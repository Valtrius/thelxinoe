#[path = "../storage/managers/controls.rs"]
mod storage;

use super::*;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/acquisition/requests/{id}/status", get(status))
        .route(
            "/api/v1/admin/acquisition/requests/{id}/releases",
            get(releases).post(grab),
        )
        .route(
            "/api/v1/admin/acquisition/requests/{id}/monitor",
            axum::routing::put(monitor),
        )
}
async fn target(state: &AppState, key: String) -> Result<(Service, Value, String)> {
    let row = storage::target(&state.db, key)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let s = service(state, &row.0).await?;
    let id = row
        .1
        .ok_or_else(|| ApiError::conflict("Request has not reached the manager yet"))?;
    let c = Connection::open(state, &s).await?;
    let item = c
        .get(&format!("{}/{id}", requests::endpoint(&s.kind)))
        .await?;
    if requests::external(&s.kind, &item).as_deref() != Some(&row.2) {
        return Err(ApiError::conflict(
            "Manager identity changed; reconcile before continuing",
        ));
    }
    Ok((s, item, row.3))
}
fn field(kind: &str) -> &'static str {
    match kind {
        "radarr" => "movieId",
        "sonarr" => "seriesId",
        _ => "albumId",
    }
}
async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    // Check ownership before making any upstream request.
    let check = key.clone();
    let uid = p.user.id.clone();
    let admin = p.user.role == thelxinoe_core::Role::Admin;
    let allowed = storage::status(&state.db, check, uid, admin).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    let (s, item, _) = target(&state, key).await?;
    let c = Connection::open(&state, &s).await?;
    let queue = c
        .call(
            reqwest::Method::GET,
            "queue",
            &[
                ("pageSize", "1000".into()),
                ("includeUnknownMovieItems", "false".into()),
                ("includeUnknownSeriesItems", "false".into()),
            ],
            None,
        )
        .await?;
    let downloads=queue["records"].as_array().into_iter().flatten().filter(|r|r[field(&s.kind)]==item["id"]).map(|r|json!({"title":r["title"],"status":r["status"],"size":r["size"],"remaining":r["sizeleft"],"time_left":r["timeleft"],"status_messages":r["statusMessages"]})).collect::<Vec<_>>();
    let available = if s.kind == "radarr" {
        item["hasFile"].as_bool().unwrap_or(false)
    } else {
        let stats = &item["statistics"];
        let (file, total) = if s.kind == "sonarr" {
            ("episodeFileCount", "episodeCount")
        } else {
            ("trackFileCount", "trackCount")
        };
        stats[total]
            .as_u64()
            .is_some_and(|n| n > 0 && stats[file].as_u64().unwrap_or(0) >= n)
    };
    Ok(Json(
        json!({"available":available,"monitored":item["monitored"],"downloads":downloads,"seasons":item["seasons"].as_array().into_iter().flatten().filter_map(|s|s["seasonNumber"].as_i64()).collect::<Vec<_>>()}),
    ))
}
#[derive(Default, Deserialize)]
struct ReleaseTarget {
    season_number: Option<i64>,
}
async fn candidates(c: &Connection<'_>, item: &Value, target: &ReleaseTarget) -> Result<Value> {
    let mut query = vec![(field(&c.kind), item["id"].to_string())];
    if c.kind == "sonarr" {
        let season = target
            .season_number
            .ok_or_else(|| ApiError::bad("Select a Sonarr season before searching releases"))?;
        if !item["seasons"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["seasonNumber"].as_i64() == Some(season))
        {
            return Err(ApiError::conflict("Season no longer exists in Sonarr"));
        }
        query.push(("seasonNumber", season.to_string()));
    }
    c.call(reqwest::Method::GET, "release", &query, None).await
}
async fn releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    axum::extract::Query(selection): axum::extract::Query<ReleaseTarget>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (s, item, _) = target(&state, key).await?;
    let c = Connection::open(&state, &s).await?;
    let rows = candidates(&c, &item, &selection).await?;
    let rows=rows.as_array().ok_or_else(unavailable)?.iter().take(500).map(|r|json!({"guid":r["guid"],"indexer_id":r["indexerId"],"title":r["title"],"size":r["size"],"quality":r["quality"],"score":r["customFormatScore"],"rejections":r["rejections"],"approved":r["approved"],"protocol":r["protocol"]})).collect::<Vec<_>>();
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
struct Grab {
    #[serde(flatten)]
    target: ReleaseTarget,
    guid: String,
    indexer_id: i64,
}
async fn grab(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Grab>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    if input.guid.len() > 2000 || input.indexer_id <= 0 {
        return Err(ApiError::bad("Select a release returned by the manager"));
    }
    let kind = target(&state, key.clone()).await?.0.kind;
    let _guard = state.managers.guard.service(&kind).await;
    let (s, item, _) = target(&state, key.clone()).await?;
    let c = Connection::open(&state, &s).await?;
    let rows = candidates(&c, &item, &input.target).await?;
    let release = rows
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .find(|r| r["guid"] == input.guid && r["indexerId"] == input.indexer_id)
        .ok_or_else(|| ApiError::conflict("Release no longer appears in the manager search"))?;
    if release["approved"] == false
        || release["rejections"]
            .as_array()
            .is_some_and(|r| !r.is_empty())
    {
        return Err(ApiError::conflict(
            "Manager rejected this release; adjust its settings before grabbing",
        ));
    }
    // No automatic retry: an interrupted grab can have reached the download client.
    c.call(
        reqwest::Method::POST,
        "release",
        &[],
        Some(json!({"guid":input.guid,"indexerId":input.indexer_id})),
    )
    .await?;
    storage::grab(&state.db, key, p).await?;
    Ok(Json(json!({"submitted":true})))
}
#[derive(Deserialize)]
struct Monitor {
    monitored: bool,
}
async fn monitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Monitor>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageServer).await?;
    let kind = target(&state, key.clone()).await?.0.kind;
    let _guard = state.managers.guard.service(&kind).await;
    let (s, mut item, _) = target(&state, key.clone()).await?;
    let c = Connection::open(&state, &s).await?;
    item["monitored"] = json!(input.monitored);
    c.call(
        reqwest::Method::PUT,
        &format!("{}/{}", requests::endpoint(&s.kind), item["id"]),
        &[],
        Some(item),
    )
    .await?;
    storage::monitor(&state.db, key, p).await?;
    Ok(Json(json!({"saved":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sonarr_release_search_requires_an_existing_manager_season() {
        let (_temp, state, _) = crate::online::oauth::tests::fixture().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/v3/release",
                    get(
                        |axum::extract::Query(query): axum::extract::Query<
                            std::collections::HashMap<String, String>,
                        >| async move {
                            assert_eq!(query.get("seriesId").unwrap(), "7");
                            assert_eq!(query.get("seasonNumber").unwrap(), "3");
                            Json(json!([{ "guid": "season-three" }]))
                        },
                    ),
                ),
            )
            .await
            .unwrap();
        });
        let connection = Connection {
            state: &state,
            base: format!("http://{address}"),
            key: "fixture".into(),
            kind: "sonarr".into(),
        };
        let series = json!({"id":7,"seasons":[{"seasonNumber":0},{"seasonNumber":3}]});
        assert!(
            candidates(&connection, &series, &ReleaseTarget::default())
                .await
                .is_err()
        );
        assert!(
            candidates(
                &connection,
                &series,
                &ReleaseTarget {
                    season_number: Some(2)
                }
            )
            .await
            .is_err()
        );
        assert_eq!(
            candidates(
                &connection,
                &series,
                &ReleaseTarget {
                    season_number: Some(3)
                }
            )
            .await
            .unwrap()[0]["guid"],
            "season-three"
        );
        task.abort();
    }
}
