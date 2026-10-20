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
#[derive(Deserialize)]
pub struct RootChange {
    name: String,
}
/// Only the name changes. Every entry's identity comes from the folder's location and
/// media type, so changing either is adding the folder again.
pub async fn rename_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<String>,
    Json(input): Json<RootChange>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    if input.name.trim().is_empty() || input.name.len() > 100 {
        return Err(ApiError::bad("Enter a library name"));
    }
    if !storage::rename_root(&state.db, p, root_id.clone(), input.name).await? {
        return Err(ApiError::not_found());
    }
    state
        .emit(None, "catalog.changed", json!({"root_id":root_id}))
        .await?;
    Ok(Json(json!({"saved":true})))
}
/// Removes a library and its entries, with everyone's state for them. Files on disk and
/// watch history stay. Media operations in progress finish first, and a running scan of
/// the library stops without writing.
pub async fn remove_root(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(root_id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageLibrary).await?;
    let _lease = state.media_operations.write().await;
    if !storage::remove_root(&state.db, p, root_id.clone()).await? {
        return Err(ApiError::not_found());
    }
    state
        .emit(None, "catalog.changed", json!({"root_id":root_id}))
        .await?;
    Ok(Json(json!({"deleted":true})))
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
    // Wanted manager titles without a library item join top-level movie and show lists.
    let wanted = input
        .kind
        .clone()
        .filter(|kind| {
            matches!(kind.as_str(), "movie" | "show")
                && input.parent.is_none()
                && input.collection.is_none()
        })
        .map(|kind| (kind, input.q.clone()));
    let mut items = storage::browse(&state.db, input).await?;
    // A full page is cut alphabetically; wanted titles past its last row wait too.
    let last = (items.len() >= 500)
        .then(|| items.last()?["title"].as_str().map(str::to_lowercase))
        .flatten();
    let grant = crate::grants::issue(&state, &p, "artwork", 300).await?;
    decorate_cards_with(
        &state,
        &p,
        &mut items.iter_mut().collect::<Vec<_>>(),
        &grant,
    )
    .await?;
    crate::managers::status::decorate(
        &state,
        &p,
        &mut items.iter_mut().collect::<Vec<_>>(),
        &grant,
    )
    .await?;
    if let Some((kind, q)) = wanted {
        items.extend(
            crate::managers::status::virtual_titles(&state, &p, &kind, q.as_deref(), &grant)
                .await?
                .into_iter()
                .filter(|item| {
                    last.as_ref().is_none_or(|last| {
                        item["title"].as_str().unwrap_or_default().to_lowercase() <= *last
                    })
                }),
        );
        // Stable, so real items keep their order when titles tie.
        items.sort_by_cached_key(|item| item["title"].as_str().unwrap_or_default().to_lowercase());
    }
    Ok(Json(json!({"items":items})))
}
/// Resolve cached art from the item, album or show in one bounded query.
pub(crate) async fn decorate_cards_with(
    state: &AppState,
    p: &thelxinoe_core::Principal,
    items: &mut [&mut Value],
    grant: &str,
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
    let p = security::require(&state, &headers, Capability::Browse).await?;
    let mut item = storage::detail(&state.db, media_id.clone())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let grant = crate::grants::issue(&state, &p, "artwork", 300).await?;
    item["artwork_url"] = Value::Null;
    decorate_cards_with(&state, &p, &mut [&mut item], &grant).await?;
    crate::managers::status::detail(&state, &p, &mut item, &grant).await?;
    // Keep protects files from deletion, which only administrators manage.
    if p.user.role == thelxinoe_core::Role::Admin {
        let protection = storage::keep(&state.db, media_id).await?;
        item["keep"] = json!(protection.own);
        item["kept_by"] = json!(protection.by);
        item["kept_within"] = json!(protection.within);
    }
    Ok(Json(item))
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
    async fn watch_later_accepts_seasons_and_clears_once_everything_is_watched() {
        let (_temp, state, alice) = fixture().await;
        state.db.write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Library','shows','/media/shows')",[])?;
            for (id,kind,parent) in [("movie","movie",None),("show","show",None),("a","season",Some("show")),("b","season",Some("show")),("a1","episode",Some("a")),("a2","episode",Some("a")),("b1","episode",Some("b"))] {
                db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES (?1,'root',?2,?3,?1,?1,1)",rusqlite::params![id,kind,parent])?;
            }
            // Specials never keep the whole series in Watch Later.
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES ('sp','root','season','show','sp','Specials',0,1),('sp1','root','episode','sp','sp1','sp1',1,1)",[])?;
            // Another user's list is never touched by Alice watching.
            db.execute("INSERT INTO media_state(user_id,media_id,updated_at,watch_later) VALUES ('bob','show',1,1),('bob','a',1,1)",[])?;
            Ok(())
        }).await.unwrap();
        let set = |media: &'static str, body: serde_json::Value| {
            let (state, alice) = (state.clone(), alice.clone());
            async move {
                let response = call(
                    &state,
                    &format!("/api/v1/catalog/{media}/state"),
                    "PUT",
                    body,
                    &alice,
                )
                .await;
                assert_eq!(response.0, axum::http::StatusCode::OK, "{media}");
                response.2
            }
        };
        let later = |user: &'static str, media: &'static str| {
            let state = state.clone();
            async move {
                state.db.read("test.later", move |db| Ok(db.query_row("SELECT COALESCE((SELECT watch_later FROM media_state WHERE user_id=?1 AND media_id=?2),0)",[user,media],|r| r.get::<_,bool>(0))?)).await.unwrap()
            }
        };
        for media in ["movie", "show", "a", "b", "a1"] {
            assert_eq!(
                set(media, serde_json::json!({"watch_later":true})).await["watch_later"],
                true
            );
        }
        assert_eq!(
            set("movie", serde_json::json!({"watched":true})).await["watch_later"],
            false
        );
        // A watched title can be queued again to rewatch; only becoming watched clears it.
        assert_eq!(
            set("movie", serde_json::json!({"watch_later":true})).await["watch_later"],
            true
        );
        assert_eq!(
            set("movie", serde_json::json!({"favorite":true})).await["watch_later"],
            true
        );
        set("a1", serde_json::json!({"watched":true})).await;
        assert!(!later("alice", "a1").await);
        assert!(later("alice", "a").await, "a2 is still unwatched");
        set("a2", serde_json::json!({"watched":true})).await;
        assert!(!later("alice", "a").await);
        assert!(later("alice", "show").await, "b1 is still unwatched");
        assert!(later("alice", "b").await);
        // Playback and Jellyfin clients write watched rows directly.
        state
            .db
            .write("test.watched", |db| {
                db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice','b1',1,2)",[])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(!later("alice", "b").await);
        assert!(!later("alice", "show").await);
        assert!(later("bob", "show").await);
        assert!(later("bob", "a").await);
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

#[cfg(test)]
mod root_tests {
    use crate::{
        AppState,
        test_support::{call, fixture},
    };
    use axum::http::StatusCode;
    use serde_json::{Value, json};

    async fn rows(state: &AppState, sql: &'static str) -> Vec<String> {
        state
            .db
            .read("test.rows", move |db| {
                Ok(db
                    .prepare(sql)?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?)
            })
            .await
            .unwrap()
    }

    // Only administrators manage library folders. Removing one takes its entries, and
    // everyone's state for them, out of the library and retires its queued work, while its
    // files, watch history and other libraries stay.
    #[tokio::test]
    async fn administrators_add_rename_and_remove_library_folders() {
        let (_temp, state, alice) = fixture().await;
        let bob =
            crate::test_support::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        let bob = format!("thelxinoe_session={bob}");
        state
            .db
            .write("test.fixture", |db| {
                db.execute("UPDATE users SET role='admin' WHERE id='alice'", [])?;
                Ok(())
            })
            .await
            .unwrap();
        for folder in ["movies", "shows"] {
            std::fs::create_dir_all(state.config.media.join(folder)).unwrap();
        }
        let film = state.config.media.join("movies/Film (2020).mkv");
        std::fs::write(&film, b"media").unwrap();
        let folder = |name: &str, kind: &str| json!({"name":name,"kind":kind,"path":state.config.media.join(kind)});
        let roots = "/api/v1/catalog/roots";
        assert_eq!(
            call(&state, roots, "POST", folder("Movies", "movies"), &bob)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let movies = call(&state, roots, "POST", folder("Movies", "movies"), &alice).await;
        assert_eq!(movies.0, StatusCode::OK);
        let movies = movies.2["id"].as_str().unwrap().to_owned();
        let shows = call(&state, roots, "POST", folder("Shows", "shows"), &alice).await;
        let shows = shows.2["id"].as_str().unwrap().to_owned();

        let path = format!("{roots}/{movies}");
        for (name, cookie, status) in [
            ("Films", &bob, StatusCode::FORBIDDEN),
            (" ", &alice, StatusCode::BAD_REQUEST),
            ("Films", &alice, StatusCode::OK),
        ] {
            let renamed = call(&state, &path, "PATCH", json!({"name":name}), cookie).await;
            assert_eq!(renamed.0, status, "{name}");
        }
        assert_eq!(
            call(
                &state,
                &format!("{roots}/unknown"),
                "PATCH",
                json!({"name":"Films"}),
                &alice
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        let listed = call(&state, roots, "GET", Value::Null, &alice).await.2;
        let names = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|root| root["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Films", "Shows"]);

        let (m, s) = (movies.clone(), shows.clone());
        state.db.write("test.fixture", move |db| {
            for (root, media, file) in [(&m, "film", "film-file"), (&s, "show", "show-file")] {
                db.execute("INSERT INTO media(id,root_id,kind,evidence_key,title,created_at) VALUES (?1,?2,'movie',?1,?1,1)", rusqlite::params![media, root])?;
                db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES (?1,?2,?1,'g',1,'1','f','{}',1)", rusqlite::params![file, root])?;
                db.execute("INSERT INTO media_sources VALUES (?1,?2)", [media, file])?;
                db.execute("INSERT INTO media_state(user_id,media_id,watched,updated_at) VALUES ('alice',?1,1,1)", [media])?;
            }
            // Rows that refer to the entries and files without cascading.
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('trailer',?1,'trailer','g',1,'1','t','{}',1)", [&m])?;
            db.execute("INSERT INTO local_trailers VALUES ('film','trailer')", [])?;
            db.execute("INSERT INTO media_operations(id,media_id,action,state,targets,created_at,updated_at) VALUES ('operation','film','delete','complete','[]',1,1)", [])?;
            db.execute("INSERT INTO retention_candidates(id,media_id,operation_id,stamp,state,eligible_at,due_at) VALUES ('candidate','film','operation','stamp','pending',1,1)", [])?;
            db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,version,checked_at) VALUES ('radarr','Radarr','radarr','radarr',7878,'g',x'00','/media','5',1)", [])?;
            db.execute("INSERT INTO manager_bindings(file_id,service_id,generation,service_generation,manager_file_id,entity_id,manager_path,external_id,members,checked_at) VALUES ('film-file','radarr','g','g',1,1,'/movies/Film.mkv','1','[]',1)", [])?;
            db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('refresh','metadata.refresh','film','{}','queued',1,1)", [])?;
            db.execute("INSERT INTO playback_history(playback_id,user_id,platform,media_id,media_title,content_type,device_name,started_at,updated_at,position,state) VALUES ('p','alice','movies','film','Film','movie','Browser',1,1,1,'stopped')", [])?;
            Ok(())
        }).await.unwrap();

        assert_eq!(
            call(&state, &path, "DELETE", Value::Null, &bob).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&state, &path, "DELETE", Value::Null, &alice).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&state, &path, "DELETE", Value::Null, &alice).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            rows(&state, "SELECT name FROM library_roots").await,
            ["Shows"]
        );
        assert_eq!(rows(&state, "SELECT id FROM media").await, ["show"]);
        assert_eq!(
            rows(&state, "SELECT id FROM media_files").await,
            ["show-file"]
        );
        assert_eq!(
            rows(&state, "SELECT media_id FROM media_state").await,
            ["show"]
        );
        assert_eq!(
            rows(&state, "SELECT media_title FROM playback_history").await,
            ["Film"]
        );
        assert!(film.exists(), "files on disk stay");
        // The new library's scan and the entry's refresh are done; the other library's
        // scan still waits.
        let s = shows.clone();
        let jobs = state
            .db
            .read("test.jobs", move |db| {
                Ok(db
                    .prepare("SELECT subject,state FROM jobs ORDER BY subject=?1,kind")?
                    .query_map([&s], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .unwrap();
        assert_eq!(
            jobs,
            [
                (movies, "complete".to_owned()),
                ("film".to_owned(), "complete".to_owned()),
                (shows, "queued".to_owned()),
            ]
        );
        assert_eq!(
            rows(
                &state,
                "SELECT action FROM audit WHERE action LIKE 'library.%' ORDER BY id"
            )
            .await,
            [
                "library.add",
                "library.add",
                "library.rename",
                "library.remove"
            ]
        );
        assert!(
            state
                .db
                .read("test.foreign_keys", |db| Ok(!db
                    .prepare("PRAGMA foreign_key_check")?
                    .exists([])?))
                .await
                .unwrap()
        );
    }

    fn wav() -> Vec<u8> {
        let samples = vec![0u8; 16000];
        let mut out = b"RIFF".to_vec();
        out.extend((36 + samples.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend([1, 0, 1, 0]);
        out.extend(8000u32.to_le_bytes());
        out.extend(16000u32.to_le_bytes());
        out.extend([2, 0, 16, 0]);
        out.extend(b"data");
        out.extend((samples.len() as u32).to_le_bytes());
        out.extend(samples);
        out
    }

    // A library removed while its scan was probing: the scan writes nothing and its job
    // completes rather than failing.
    #[tokio::test]
    async fn a_scan_of_a_library_removed_while_it_ran_completes_without_writing() {
        let (_temp, state, _) = fixture().await;
        crate::test_support::tool_runtime::install(
            &state.tools.runtime,
            &state.config.state.join("tools"),
        );
        let folder = state.config.media.join("music");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Song.wav"), wav()).unwrap();
        let path = folder.to_string_lossy().into_owned();
        state.db.write("test.fixture", move |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('root','Music','music',?1)", [path])?;
            db.execute("INSERT INTO jobs(id,kind,subject,payload,state,available_at,created_at) VALUES ('scan','library.scan','root','{\"root_id\":\"root\"}','queued',1,1)", [])?;
            Ok(())
        }).await.unwrap();
        let queue = thelxinoe_jobs::Queue(state.db.clone());
        let job = queue.claim_general().await.unwrap().unwrap();
        // The scan probes the folder, then waits for the media lease to write.
        let lease = state.media_operations.write().await;
        let scan = tokio::spawn({
            let state = state.clone();
            async move { crate::run_job(state, job).await }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !state.db.read("test.probed", |db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE kind='catalog.scan.progress' AND json_extract(payload,'$.completed')=1)", [], |r| r.get::<_, bool>(0))?)).await.unwrap() {
            assert!(std::time::Instant::now() < deadline, "the scan never probed");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let admin = thelxinoe_core::Principal {
            user: thelxinoe_core::User {
                id: "alice".into(),
                username: "alice".into(),
                role: thelxinoe_core::Role::Admin,
                timezone: "UTC".into(),
            },
            session_id: "session".into(),
            transport: "web".into(),
        };
        assert!(
            super::storage::remove_root(&state.db, admin, "root".into())
                .await
                .unwrap()
        );
        drop(lease);
        scan.await.unwrap();
        assert_eq!(
            rows(
                &state,
                "SELECT state||':'||COALESCE(error,'') FROM jobs WHERE id='scan'"
            )
            .await,
            ["complete:"]
        );
        assert!(rows(&state, "SELECT id FROM media_files").await.is_empty());
    }
}
