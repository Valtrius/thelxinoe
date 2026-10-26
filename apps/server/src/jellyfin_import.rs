//! Brings watched state, resume points and favorites over from an existing Jellyfin server.
//! Jellyfin is only read. Imports add state and never remove any, so running one again
//! during a move only adds what was watched since.

mod client;
mod matching;
#[path = "storage/jellyfin_import.rs"]
mod storage;
#[cfg(test)]
mod tests;

use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post, put},
};
use client::{Filter, Item, Jellyfin, Server};
use matching::{Group, Matcher, Outcome};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use storage::{Applied, Entry};
use thelxinoe_core::{Capability, now};

/// Rows listing what was not found stop here; the counts stay complete.
const MISSING_ROWS: usize = 1000;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/jellyfin-import", get(overview).post(import))
        .route(
            "/api/v1/admin/jellyfin-import/server",
            put(connect).delete(forget),
        )
        .route("/api/v1/admin/jellyfin-import/accounts", get(accounts))
        .route("/api/v1/admin/jellyfin-import/preview", post(preview))
}

fn not_connected() -> ApiError {
    ApiError::conflict("Connect a Jellyfin server first")
}

async fn saved(state: &AppState) -> Result<Option<Server>> {
    let Some(bytes) = state.secrets.get(&state.db, storage::SECRET).await? else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_slice(&bytes).map_err(anyhow::Error::from)?,
    ))
}

fn shown(server: &Server) -> Value {
    json!({"url":server.url,"name":server.name,"version":server.version})
}

async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let mut value = storage::overview(&state.db).await?;
    value["server"] = saved(&state).await?.as_ref().map_or(Value::Null, shown);
    Ok(Json(value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    url: String,
    api_key: String,
}

async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Connect>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    crate::authentication::require_fresh(&state, &p).await?;
    let url = client::address(&input.url)?;
    let (name, version) = Jellyfin::new(&url, &input.api_key)?.identify().await?;
    let server = Server {
        url,
        api_key: input.api_key.trim().to_owned(),
        name,
        version,
    };
    let encrypted = state.secrets.encrypt(
        storage::SECRET,
        &serde_json::to_vec(&server).map_err(anyhow::Error::from)?,
    )?;
    storage::connect(&state.db, p.user.id, encrypted).await?;
    Ok(Json(shown(&server)))
}

async fn forget(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    storage::forget(&state.db, p.user.id).await?;
    Ok(Json(json!({"saved":true})))
}

/// Jellyfin's accounts with what each has to import, and who here matches each by name.
async fn accounts(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let server = saved(&state).await?.ok_or_else(not_connected)?;
    let jellyfin = Jellyfin::new(&server.url, &server.api_key)?;
    let people = storage::people(&state.db).await?;
    let mut accounts = Vec::new();
    for account in jellyfin.accounts().await? {
        let (played, in_progress, favorites) = tokio::try_join!(
            jellyfin.count(&account.id, Filter::Played),
            jellyfin.count(&account.id, Filter::Resumable),
            jellyfin.count(&account.id, Filter::Favorite),
        )?;
        let suggested = people
            .iter()
            .find(|(_, username)| username.to_lowercase() == account.name.to_lowercase())
            .map(|(id, _)| id);
        accounts.push(json!({
            "id":account.id,"name":account.name,
            "administrator":account.policy.is_administrator,"disabled":account.policy.is_disabled,
            "played":played,"in_progress":in_progress,"favorites":favorites,"suggested":suggested,
        }));
    }
    let people: Vec<Value> = people
        .iter()
        .map(|(id, username)| json!({"id":id,"username":username}))
        .collect();
    Ok(Json(json!({"accounts":accounts,"people":people})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    jellyfin_id: String,
    user_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    accounts: Vec<Mapping>,
    watched: bool,
    resume: bool,
    favorites: bool,
    /// Title-matched groups the administrator left out.
    #[serde(default)]
    excluded: Vec<String>,
}

/// Who is imported, and what the report lists besides the counts.
#[derive(Clone)]
struct Meta {
    server: String,
    /// Jellyfin ID and name, and the person here.
    accounts: Vec<(String, String, String, String)>,
    groups: Vec<Group>,
    missing: Vec<Value>,
    missing_counts: Vec<usize>,
}

/// Reads each account's history from Jellyfin and decides where every item goes.
async fn plan(state: &AppState, input: &Request) -> Result<(Meta, Vec<Entry>)> {
    if input.accounts.is_empty() || input.accounts.len() > 100 {
        return Err(ApiError::bad("Choose the Jellyfin accounts to import"));
    }
    if !(input.watched || input.resume || input.favorites) {
        return Err(ApiError::bad("Choose what to import"));
    }
    if input.excluded.len() > 10_000 {
        return Err(ApiError::bad("Too many titles left out"));
    }
    let server = saved(state).await?.ok_or_else(not_connected)?;
    let jellyfin = Jellyfin::new(&server.url, &server.api_key)?;
    let known: HashMap<String, String> = jellyfin
        .accounts()
        .await?
        .into_iter()
        .map(|account| (account.id, account.name))
        .collect();
    let people: HashMap<String, String> = storage::people(&state.db).await?.into_iter().collect();
    let mut accounts = Vec::new();
    let mut seen = HashSet::new();
    for mapping in &input.accounts {
        let (Some(name), Some(username)) = (
            known.get(&mapping.jellyfin_id),
            people.get(&mapping.user_id),
        ) else {
            return Err(ApiError::bad("An account to import no longer exists"));
        };
        if !seen.insert(&mapping.jellyfin_id) {
            return Err(ApiError::bad("Each Jellyfin account can be imported once"));
        }
        accounts.push((
            mapping.jellyfin_id.clone(),
            name.clone(),
            mapping.user_id.clone(),
            username.clone(),
        ));
    }

    let mut histories: Vec<Vec<Item>> = Vec::new();
    for (jellyfin_id, ..) in &accounts {
        // Every item carries its whole user data, so one appearing in several lists is read once.
        let mut items: Vec<Item> = Vec::new();
        let mut ids = HashSet::new();
        for (wanted, filter) in [
            (input.watched, Filter::Played),
            (input.resume, Filter::Resumable),
            (input.favorites, Filter::Favorite),
        ] {
            if !wanted {
                continue;
            }
            for item in jellyfin.items(jellyfin_id, filter).await? {
                if ids.insert(item.id.clone()) {
                    items.push(item);
                }
            }
        }
        histories.push(items);
    }
    let fetched: HashSet<&str> = histories
        .iter()
        .flatten()
        .filter(|item| item.kind == "Series")
        .map(|item| item.id.as_str())
        .collect();
    let mut series_ids: Vec<String> = histories
        .iter()
        .flatten()
        .filter_map(|item| item.series_id.clone())
        .filter(|id| !fetched.contains(id.as_str()))
        .collect();
    series_ids.sort();
    series_ids.dedup();
    let series = if series_ids.is_empty() {
        Vec::new()
    } else {
        jellyfin.series(&accounts[0].0, &series_ids).await?
    };
    let catalog = storage::catalog(&state.db).await?;
    let excluded = input.excluded.clone();
    let (watched, resume, favorites) = (input.watched, input.resume, input.favorites);
    let names = accounts.clone();
    // Matching a large library is CPU work; keep it off the request threads.
    let (entries, groups, missing, missing_counts) = tokio::task::spawn_blocking(move || {
        let matcher = Matcher::new(&catalog);
        let (outcomes, groups) = matcher.resolve(&histories, &series, &excluded);
        let mut entries = Vec::new();
        let mut missing: BTreeMap<(usize, String), Value> = BTreeMap::new();
        let mut missing_counts = vec![0; histories.len()];
        for (account, (items, outcomes)) in histories.iter().zip(outcomes).enumerate() {
            for (item, outcome) in items.iter().zip(outcomes) {
                let playable = item.kind != "Series";
                let state = &item.user_data;
                let played = watched && state.played && playable;
                let favorite = favorites && state.is_favorite;
                let position =
                    (resume && playable && !state.played && state.playback_position_ticks > 0)
                        .then(|| {
                            (
                                state.playback_position_ticks as f64 / 10_000_000.0,
                                item.last_played(),
                            )
                        });
                if !(played || favorite || position.is_some()) {
                    continue;
                }
                match outcome {
                    Outcome::Found(target) => entries.push(Entry {
                        account,
                        user: names[account].2.clone(),
                        target,
                        watched: played,
                        favorite,
                        resume: position,
                    }),
                    Outcome::Excluded => {}
                    Outcome::Missing => {
                        missing_counts[account] += 1;
                        let episode = item.kind == "Episode";
                        let (key, title, year) = if episode {
                            (
                                item.series_id.clone().unwrap_or_else(|| item.id.clone()),
                                item.series_name
                                    .clone()
                                    .unwrap_or_else(|| item.name.clone()),
                                None,
                            )
                        } else {
                            (item.id.clone(), item.name.clone(), item.production_year)
                        };
                        let row = missing.entry((account, key)).or_insert_with(|| {
                            json!({"account":names[account].1,"title":title,"year":year,
                                "episodes":if item.kind == "Movie" {Value::Null} else {json!(0)},
                                "watched":0,"resume":0,"favorite":0,"position":null})
                        });
                        let add = |row: &mut Value, field: &str| {
                            row[field] = json!(row[field].as_u64().unwrap_or(0) + 1);
                        };
                        if episode {
                            add(row, "episodes");
                        }
                        if played {
                            add(row, "watched");
                        }
                        if let Some((seconds, _)) = position {
                            add(row, "resume");
                            row["position"] = json!(seconds.round());
                        }
                        if favorite {
                            add(row, "favorite");
                        }
                    }
                }
            }
        }
        let mut missing: Vec<Value> = missing.into_values().collect();
        missing.sort_by(|a, b| {
            (a["account"].as_str(), a["title"].as_str())
                .cmp(&(b["account"].as_str(), b["title"].as_str()))
        });
        missing.truncate(MISSING_ROWS);
        (entries, groups, missing, missing_counts)
    })
    .await
    .map_err(anyhow::Error::from)?;
    Ok((
        Meta {
            server: server.name,
            accounts,
            groups,
            missing,
            missing_counts,
        },
        entries,
    ))
}

fn report(meta: &Meta, applied: &Applied, preview: bool) -> Value {
    let accounts: Vec<Value> = meta
        .accounts
        .iter()
        .zip(&applied.accounts)
        .zip(&meta.missing_counts)
        .map(
            |(((jellyfin_id, name, user_id, username), counts), missing)| {
                json!({"jellyfin_id":jellyfin_id,"name":name,"user_id":user_id,"username":username,
                "watched":counts.watched,"already_watched":counts.already_watched,
                "resume":counts.resume,"favorites":counts.favorites,"not_found":missing})
            },
        )
        .collect();
    let mut report = json!({
        "at":now(),"server":meta.server,"accounts":accounts,
        "not_found":meta.missing,"retention":applied.retention,
    });
    if preview {
        report["groups"] = json!(
            meta.groups
                .iter()
                .map(|group| json!({
                    "key":group.key,"jellyfin":group.jellyfin,"title":group.title,
                    "series":group.series,"items":group.items,
                    "accounts":applied.groups.get(&group.key),
                }))
                .collect::<Vec<_>>()
        );
    }
    report
}

/// Counts what an import would change without changing anything.
async fn preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (meta, entries) = plan(&state, &input).await?;
    let _lease = state.media_operations.read().await;
    let applied = storage::apply(&state.db, meta.accounts.len(), entries, None).await?;
    Ok(Json(report(&meta, &applied, true)))
}

async fn import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    crate::authentication::require_fresh(&state, &p).await?;
    let (meta, entries) = plan(&state, &input).await?;
    let saved = meta.clone();
    let applied = {
        let _lease = state.media_operations.read().await;
        storage::apply(
            &state.db,
            meta.accounts.len(),
            entries,
            Some((
                p.user.id.clone(),
                Box::new(move |applied: &Applied| report(&saved, applied, false)),
            )),
        )
        .await?
    };
    for user in &applied.users {
        state
            .emit(Some(user.clone()), "media-state.changed", json!({}))
            .await?;
    }
    Ok(Json(report(&meta, &applied, false)))
}
