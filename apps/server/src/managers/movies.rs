//! Removing a Radarr movie that has no library files. The movie is addressed like its
//! title page, by library id or `tmdb:` reference.

#[path = "../storage/managers/movies.rs"]
mod storage;

use super::domain::RadarrOptions;
use super::*;
use axum::extract::Query;

pub(super) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/v1/admin/movies/{id}",
        axum::routing::delete(remove_movie),
    )
}

#[derive(Deserialize)]
struct Removal {
    #[serde(default)]
    exclude: bool,
}

async fn remove_movie(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<Removal>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let movie = super::status::movie(&state, &id).await?;
    let _lease = state.media_operations.write().await;
    if let Some(media) = &movie.media {
        if !operations::targets(&state, media).await?.is_empty() {
            return Err(ApiError::conflict(
                "This movie still has files; delete them first",
            ));
        }
        operations::protected_or_active(&state, media, &[]).await?;
    }
    let _guard = state.managers.guard.service("radarr").await;
    let s = service(&state, &movie.service).await?;
    let c = Connection::open(&state, &s).await?;
    let options = RadarrOptions {
        remove: true,
        exclude: input.exclude,
    };
    apply(&c, movie.id, &movie.tmdb, options).await?;
    audit(&state, Some(p.user.id), options, id).await?;
    super::status::refresh_soon(&state);
    Ok(Json(json!({"removed":true})))
}

/// Exclude and remove a Radarr movie without a file, as `options` ask.
pub(super) async fn apply(
    c: &Connection<'_>,
    entity: i64,
    tmdb: &str,
    options: RadarrOptions,
) -> Result<()> {
    if options.exclude {
        super::retention::ensure_exclusion(c, entity, tmdb).await?;
    }
    if options.remove {
        remove(c, entity, tmdb).await?;
    }
    Ok(())
}

pub(super) async fn audit(
    state: &AppState,
    actor: Option<String>,
    options: RadarrOptions,
    movie: String,
) -> Result<()> {
    let action = match (options.remove, options.exclude) {
        (true, true) => "movie.remove.exclude",
        (true, false) => "movie.remove",
        _ => "movie.exclude",
    };
    storage::audit(&state.db, actor, action, movie).await?;
    Ok(())
}

/// Remove a movie from Radarr once it is confirmed to be the same movie, without a file.
/// Its folder stays: Radarr would delete everything in it.
async fn remove(c: &Connection<'_>, entity: i64, tmdb: &str) -> Result<()> {
    let movie = c.get(&format!("movie/{entity}")).await?;
    if requests::external("radarr", &movie).as_deref() != Some(tmdb) {
        return Err(ApiError::conflict("Radarr movie identity changed"));
    }
    if movie["hasFile"] != false {
        return Err(ApiError::conflict("Radarr still has a file for this movie"));
    }
    c.call(
        reqwest::Method::DELETE,
        &format!("movie/{entity}"),
        &[
            ("deleteFiles", "false".into()),
            ("addImportExclusion", "false".into()),
        ],
        None,
    )
    .await
    .map(drop)
}

#[cfg(test)]
#[path = "movies_tests.rs"]
mod tests;
