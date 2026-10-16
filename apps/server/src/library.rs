#[path = "storage/library.rs"]
mod storage;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use thelxinoe_core::{Capability, id, now};

pub async fn roots(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    let watching = state
        .library_watch
        .lock()
        .map(|watching| watching.clone())
        .unwrap_or_default();
    let mut items = json!(thelxinoe_catalog::roots(&state.db).await?);
    for item in items.as_array_mut().into_iter().flatten() {
        item["watching"] = json!(item["id"].as_str().and_then(|id| watching.get(id)));
    }
    Ok(Json(
        json!({"items":items,"media_mount":state.config.media}),
    ))
}
#[derive(Deserialize)]
pub struct NewRoot {
    name: String,
    kind: String,
    path: String,
}
pub async fn add_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewRoot>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || !matches!(input.kind.as_str(), "movies" | "shows" | "music")
    {
        return Err(ApiError::bad("Enter a library name and valid media type"));
    }
    let path =
        thelxinoe_catalog::approved_path(std::path::Path::new(&input.path), &state.config.media)
            .map_err(|e| ApiError::bad(e.to_string()))?;
    let root_id = id();
    let rid = root_id.clone();
    let path = path.to_string_lossy().to_string();
    let added = storage::add_root(&state.db, input, p, rid, path).await?;
    if !added {
        return Err(ApiError::conflict("Library roots cannot overlap"));
    }
    enqueue_scan(&state, &root_id).await?;
    Ok(Json(json!({"id":root_id})))
}
pub async fn enqueue_scan(state: &AppState, root_id: &str) -> anyhow::Result<String> {
    let root_id = root_id.to_string();
    storage::enqueue_scan(root_id, &state.db).await
}
pub async fn scan(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageLibrary).await?;
    if !thelxinoe_catalog::roots(&state.db)
        .await?
        .iter()
        .any(|r| r.id == root_id)
    {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"job_id":enqueue_scan(&state,&root_id).await?})))
}
#[derive(Deserialize, Default)]
pub struct Browse {
    kind: Option<String>,
    parent: Option<String>,
    q: Option<String>,
    collection: Option<i64>,
}
pub async fn browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Browse>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::Browse).await?;
    let mut items = storage::browse(&state.db, input).await?;
    decorate_cards(&state, &p, &mut items.iter_mut().collect::<Vec<_>>()).await?;
    Ok(Json(json!({"items":items})))
}
/// Resolve cached art from the item, album or show in one bounded query.
pub(crate) async fn decorate_cards(
    state: &AppState,
    p: &thelxinoe_core::Principal,
    items: &mut [&mut Value],
) -> Result<()> {
    let ids = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let ids = json!(ids).to_string();
    let metadata = storage::decorate_cards(ids, p.user.id.clone(), &state.db).await?;
    let grant = crate::grants::issue(state, p, "artwork", 300).await?;
    for item in items {
        if let Some(presentation) = item["id"].as_str().and_then(|id| metadata.get(id)) {
            if item.get("year").is_none() {
                item["year"] = json!(presentation.year);
            }
            item["watched"] = json!(presentation.watched);
            if let Some(id) = &presentation.artwork_id {
                item["artwork_url"] = json!(format!("/api/v1/catalog/{id}/artwork?grant={grant}"));
            }
        }
    }
    Ok(())
}

pub async fn detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::Browse).await?;
    let result = storage::detail(&state.db, media_id).await?;
    Ok(Json(result.ok_or_else(ApiError::not_found)?))
}
pub async fn overrides(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(media_id): Path<String>,
    Json(value): Json<Value>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let Some(fields) = value.as_object() else {
        return Err(ApiError::bad("Expected metadata fields"));
    };
    if fields.iter().any(|(key, value)| match key.as_str() {
        "title" | "overview" => !value.is_null() && value.as_str().is_none_or(|s| s.len() > 10_000),
        "year" => !value.is_null() && !value.as_i64().is_some_and(|n| (1..=9999).contains(&n)),
        _ => true,
    }) {
        return Err(ApiError::bad("Unsupported metadata override"));
    }
    let updated = storage::overrides(&state.db, media_id, value, p).await?;
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({"ok":true})))
}
/// A root is rescanned once its changes pause this long, so a burst becomes one scan. A
/// file still being written is skipped by that scan and its next write triggers another.
const QUIET: std::time::Duration = std::time::Duration::from_secs(5);
/// Continuous activity is still scanned at least this often.
const BUSY: std::time::Duration = std::time::Duration::from_secs(600);
/// Full rescans catch anything missed: rarely while live updates work, more often without.
const WATCHED: std::time::Duration = std::time::Duration::from_secs(3600);
const UNWATCHED: std::time::Duration = std::time::Duration::from_secs(900);
/// The first and latest change seen for each root since it was last scanned.
type Activity = std::sync::Arc<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, std::time::Instant)>>,
>;
struct Watch {
    path: String,
    watcher: Option<notify::RecommendedWatcher>,
    attempted: std::time::Instant,
}
pub async fn reconcile(state: AppState) -> anyhow::Result<()> {
    let activity = Activity::default();
    // One watcher per library root, never the whole media mount: downloads and other
    // folders don't trigger scans or spend the system's file-watch budget.
    let mut watches = std::collections::HashMap::<String, Watch>::new();
    let mut scanned = std::collections::HashMap::<String, std::time::Instant>::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
    // Startup catches up on every root; a root added later is scanned by whoever added it.
    let mut started = false;
    loop {
        tick.tick().await;
        let roots = thelxinoe_catalog::roots(&state.db).await?;
        watches.retain(|id, w| roots.iter().any(|r| &r.id == id && r.path == w.path));
        scanned.retain(|id, _| roots.iter().any(|r| &r.id == id));
        for root in &roots {
            let entry = watches.entry(root.id.clone()).or_insert_with(|| Watch {
                path: root.path.clone(),
                watcher: watch(&root.id, &root.path, &activity),
                attempted: std::time::Instant::now(),
            });
            if entry.watcher.is_none() && entry.attempted.elapsed() >= UNWATCHED {
                entry.watcher = watch(&root.id, &root.path, &activity);
                entry.attempted = std::time::Instant::now();
            }
            let live = entry.watcher.is_some();
            if let Ok(mut watching) = state.library_watch.lock() {
                watching.insert(root.id.clone(), live);
            }
            let changed = activity
                .lock()
                .ok()
                .and_then(|activity| activity.get(&root.id).copied());
            if started {
                scanned
                    .entry(root.id.clone())
                    .or_insert_with(std::time::Instant::now);
            }
            let last = scanned.get(&root.id).copied();
            if due(changed, last, live, std::time::Instant::now()) {
                if let Ok(mut activity) = activity.lock() {
                    activity.remove(&root.id);
                }
                scanned.insert(root.id.clone(), std::time::Instant::now());
                enqueue_scan(&state, &root.id).await?;
            }
        }
        if let Ok(mut watching) = state.library_watch.lock() {
            watching.retain(|id, _| roots.iter().any(|r| &r.id == id));
        }
        started = true;
    }
}
/// A root needs a scan once its changes settle, or when its periodic refresh is due.
fn due(
    changed: Option<(std::time::Instant, std::time::Instant)>,
    scanned: Option<std::time::Instant>,
    live: bool,
    now: std::time::Instant,
) -> bool {
    let settled = changed.is_some_and(|(first, last)| {
        now.duration_since(last) >= QUIET || now.duration_since(first) >= BUSY
    });
    let refresh = if live { WATCHED } else { UNWATCHED };
    settled || scanned.is_none_or(|at| now.duration_since(at) >= refresh)
}
fn watch(id: &str, path: &str, activity: &Activity) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let (key, sink) = (id.to_owned(), activity.clone());
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| {
            matches!(
                event.kind,
                notify::EventKind::Create(_)
                    | notify::EventKind::Modify(_)
                    | notify::EventKind::Remove(_)
                    | notify::EventKind::Any
            )
        }) && let Ok(mut activity) = sink.lock()
        {
            let now = std::time::Instant::now();
            activity
                .entry(key.clone())
                .and_modify(|changed| changed.1 = now)
                .or_insert((now, now));
        }
    })
    .ok()?;
    match watcher.watch(std::path::Path::new(path), notify::RecursiveMode::Recursive) {
        Ok(()) => Some(watcher),
        Err(error) => {
            // Dropping the watcher releases every watch added before the failure, so the
            // system's budget stays available to media servers running as the same user.
            tracing::warn!(%error, root = id, "Live library updates unavailable; rescanning every 15 minutes");
            None
        }
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn roots_scan_after_changes_settle_or_their_refresh_is_due() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        // Changes wait for quiet, but continuous activity is still scanned.
        assert!(!due(Some((at(0), at(20))), Some(at(0)), true, at(24)));
        assert!(due(Some((at(0), at(20))), Some(at(0)), true, at(25)));
        assert!(due(Some((at(0), at(599))), Some(at(0)), true, at(600)));
        // Without changes: never scanned, or the refresh for the watch state is due.
        assert!(due(None, None, true, at(0)));
        assert!(!due(None, Some(at(0)), true, at(900)));
        assert!(due(None, Some(at(0)), false, at(900)));
        assert!(due(None, Some(at(0)), true, at(3600)));
    }
    #[test]
    fn a_root_watch_records_changes_and_an_unwatchable_root_holds_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let activity = Activity::default();
        let path = directory.path().to_string_lossy().into_owned();
        let watcher = watch("root", &path, &activity).expect("watchable root");
        std::fs::create_dir(directory.path().join("Movie (2024)")).unwrap();
        std::fs::write(directory.path().join("Movie (2024)/movie.mkv"), b"media").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !activity.lock().unwrap().contains_key("root") {
            assert!(Instant::now() < deadline, "no change was recorded");
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(watcher);
        let missing = directory
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned();
        assert!(watch("missing", &missing, &activity).is_none());
    }
}

#[cfg(test)]
mod presentation_tests {
    use crate::test_support::{call, fixture};
    use serde_json::Value;
    #[tokio::test]
    async fn cards_expose_only_the_viewers_watched_state_and_completed_tv_containers() {
        let (_temp, state, alice) = fixture().await;
        state.db.write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Library','shows','/media/shows')",[])?;
            for (id,kind,parent) in [("movie","movie",None),("show","show",None),("season","season",Some("show")),("empty","season",Some("show")),("episode1","episode",Some("season")),("episode2","episode",Some("season"))] {
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'root',?2,?3,?1,?1,1)",rusqlite::params![id,kind,parent])?;
            }
            for media in ["movie", "episode1", "episode2"] {
                db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite) VALUES ('alice',?1,1,1,1)",[media])?;
            }
            Ok(())
        }).await.unwrap();
        let cards = call(&state, "/api/v1/catalog", "GET", Value::Null, &alice).await;
        assert_eq!(cards.0, axum::http::StatusCode::OK);
        for item in cards.2["items"].as_array().unwrap() {
            assert_eq!(item["watched"], item["id"] != "empty", "{item}");
        }
        let home = call(&state, "/api/v1/me/home", "GET", Value::Null, &alice).await;
        assert!(
            home.2["favorites"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["watched"] == true)
        );
        let bob =
            crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        let bob_cards = call(
            &state,
            "/api/v1/catalog",
            "GET",
            Value::Null,
            &format!("thelxinoe_session={bob}"),
        )
        .await;
        assert!(
            bob_cards.2["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item["watched"] == false)
        );
        let changed = call(
            &state,
            "/api/v1/catalog/episode2/state",
            "PUT",
            serde_json::json!({"watched":false}),
            &alice,
        )
        .await;
        assert_eq!(changed.0, axum::http::StatusCode::OK);
        for kind in ["show", "season"] {
            let cards = call(
                &state,
                &format!("/api/v1/catalog?kind={kind}"),
                "GET",
                Value::Null,
                &alice,
            )
            .await;
            assert!(
                cards.2["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["watched"] == false)
            );
        }
    }

    #[tokio::test]
    async fn home_inherits_show_artwork_without_exposing_other_users_shelves() {
        let (_temp, state, alice) = fixture().await;
        state.db.write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Shows','shows','/media/shows')",[])?;
            for (id,kind,parent,metadata) in [("show","show",None,"{\"artwork_cached\":true}"),("season","season",Some("show"),"{}"),("episode","episode",Some("season"),"{}")] {
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,metadata,created_at) VALUES (?1,'root',?2,?3,?1,?1,?4,1)",rusqlite::params![id,kind,parent,metadata])?;
            }
            db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at,favorite) VALUES ('alice','episode',0,1,1)",[])?;
            Ok(())
        }).await.unwrap();
        let response = call(&state, "/api/v1/me/home", "GET", Value::Null, &alice).await;
        assert_eq!(response.0, axum::http::StatusCode::OK);
        let url = response.2["favorites"][0]["artwork_url"].as_str().unwrap();
        assert!(url.starts_with("/api/v1/catalog/show/artwork?grant="));
        let bob =
            crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        assert_eq!(
            call(
                &state,
                "/api/v1/me/home",
                "GET",
                Value::Null,
                &format!("thelxinoe_session={bob}")
            )
            .await
            .2["favorites"],
            serde_json::json!([])
        );
    }
}
