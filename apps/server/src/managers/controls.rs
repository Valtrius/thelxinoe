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
    let row=state.db.call(move|db|Ok(db.query_row("SELECT service_id,manager_id,external_id,user_id FROM acquisition_requests WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?)).await?.ok_or_else(ApiError::not_found)?;
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
    let allowed=state.db.call(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM acquisition_requests WHERE id=?1 AND (?2 OR user_id=?3))",params![check,admin,uid],|r|r.get::<_,bool>(0))?)).await?;
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
        json!({"available":available,"monitored":item["monitored"],"downloads":downloads}),
    ))
}
async fn candidates(c: &Connection<'_>, item: &Value) -> Result<Value> {
    c.call(
        reqwest::Method::GET,
        "release",
        &[(field(&c.kind), item["id"].to_string())],
        None,
    )
    .await
}
async fn releases(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let (s, item, _) = target(&state, key).await?;
    let c = Connection::open(&state, &s).await?;
    let rows = candidates(&c, &item).await?;
    let rows=rows.as_array().ok_or_else(unavailable)?.iter().take(500).map(|r|json!({"guid":r["guid"],"indexer_id":r["indexerId"],"title":r["title"],"size":r["size"],"quality":r["quality"],"score":r["customFormatScore"],"rejections":r["rejections"],"approved":r["approved"],"protocol":r["protocol"]})).collect::<Vec<_>>();
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
struct Grab {
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
    let _guard = state.managers.guard.lock().await;
    let (s, item, _) = target(&state, key.clone()).await?;
    let c = Connection::open(&state, &s).await?;
    let rows = candidates(&c, &item).await?;
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
    state.db.call(move|db|{db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.grab',?2,?3)",params![p.user.id,key,now()])?;Ok(())}).await?;
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
    let _guard = state.managers.guard.lock().await;
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
    state.db.call(move|db|{db.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.monitor',?2,?3)",params![p.user.id,key,now()])?;Ok(())}).await?;
    Ok(Json(json!({"saved":true})))
}
