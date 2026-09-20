use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/managers/reconcile", post(reconcile_api))
        .route("/api/v1/admin/managers/bindings", get(list))
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Claim {
    pub service_id: String,
    pub service_generation: String,
    pub manager_file_id: i64,
    pub entity_id: i64,
    pub manager_path: String,
    pub external_id: String,
    // Exact manager episode/track IDs; never derived from displayed numbering.
    pub members: Vec<i64>,
    pub server_path: String,
}

fn positive(row: &Value, field: &str) -> Result<i64> {
    row[field]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)
}
pub(super) async fn inventory(state: &AppState, s: &Service) -> Result<Vec<Claim>> {
    let c = Connection::open(state, s).await?;
    let entities = c.get(requests::endpoint(&s.kind)).await?;
    let mut claims = Vec::new();
    for entity in entities.as_array().ok_or_else(unavailable)? {
        if (s.kind == "radarr" && entity["hasFile"] == false)
            || (s.kind == "sonarr" && entity["statistics"]["episodeFileCount"].as_u64() == Some(0))
            || (s.kind == "lidarr" && entity["statistics"]["trackFileCount"].as_u64() == Some(0))
        {
            continue;
        }
        let entity_id = positive(entity, "id")?;
        let external_id = requests::external(&s.kind, entity).ok_or_else(unavailable)?;
        let (files, members) = match s.kind.as_str() {
            "radarr" => (
                c.call(
                    reqwest::Method::GET,
                    "moviefile",
                    &[("movieId", entity_id.to_string())],
                    None,
                )
                .await?,
                json!([]),
            ),
            "sonarr" => (
                c.call(
                    reqwest::Method::GET,
                    "episodefile",
                    &[("seriesId", entity_id.to_string())],
                    None,
                )
                .await?,
                c.call(
                    reqwest::Method::GET,
                    "episode",
                    &[("seriesId", entity_id.to_string())],
                    None,
                )
                .await?,
            ),
            _ => (
                c.call(
                    reqwest::Method::GET,
                    "trackfile",
                    &[("albumId", entity_id.to_string())],
                    None,
                )
                .await?,
                c.call(
                    reqwest::Method::GET,
                    "track",
                    &[("albumId", entity_id.to_string())],
                    None,
                )
                .await?,
            ),
        };
        for file in files.as_array().ok_or_else(unavailable)? {
            let manager_file_id = positive(file, "id")?;
            let path = file["path"].as_str().ok_or_else(unavailable)?;
            if !clean_path(path) || suffix(path, canonical_root(&s.kind)).is_none() {
                return Err(ApiError::conflict(
                    "Manager files must use the canonical /media library path; update existing paths in the service's bulk editor",
                ));
            }
            let server_path = path.to_owned();
            let field = if s.kind == "sonarr" {
                "episodeFileId"
            } else {
                "trackFileId"
            };
            let members = members
                .as_array()
                .ok_or_else(unavailable)?
                .iter()
                .filter(|m| m[field].as_i64() == Some(manager_file_id))
                .map(|m| positive(m, "id"))
                .collect::<Result<Vec<_>>>()?;
            if s.kind != "radarr" && members.is_empty() {
                return Err(ApiError::conflict(
                    "Manager file has no exact episode or track identity",
                ));
            }
            claims.push(Claim {
                service_id: s.id.clone(),
                service_generation: s.generation.clone(),
                manager_file_id,
                entity_id,
                manager_path: path.into(),
                external_id: external_id.clone(),
                members,
                server_path,
            });
        }
    }
    Ok(claims)
}
pub(super) async fn reconcile(state: &AppState) -> Result<()> {
    let services = state
        .db
        .call(|db| {
            Ok(db
                .prepare("SELECT id FROM manager_services WHERE enabled=1 ORDER BY id")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    for key in services {
        let s = service(state, &key).await?;
        let result = inventory(state, &s).await;
        let generation = s.generation;
        // Failed reconciliation preserves every historical claim.
        state.db.call(move|db| {
            let tx=db.transaction()?;
            match result {
                Ok(claims)=> {
                    let mut current=Vec::new();
                    for claim in claims {
                        let file=tx.query_row("SELECT id,generation FROM media_files WHERE path=?1 AND present=1",[&claim.server_path],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
                        if let Some((file,generation))=file {
                            current.push(file.clone());
                            tx.execute("INSERT INTO manager_bindings VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(file_id,service_id) DO UPDATE SET generation=excluded.generation,service_generation=excluded.service_generation,manager_file_id=excluded.manager_file_id,entity_id=excluded.entity_id,manager_path=excluded.manager_path,external_id=excluded.external_id,members=excluded.members,checked_at=excluded.checked_at",params![file,claim.service_id,generation,claim.service_generation,claim.manager_file_id,claim.entity_id,claim.manager_path,claim.external_id,serde_json::to_string(&claim.members)?,now()])?;
                        }
                    }
                    // Retain old rows as historical evidence, but make freshness explicit.
                    tx.execute("UPDATE manager_bindings SET checked_at=0 WHERE service_id=?1 AND file_id NOT IN (SELECT value FROM json_each(?2))",params![key,serde_json::to_string(&current)?])?;
                    tx.execute("INSERT INTO manager_reconciliations VALUES (?1,?2,?3,NULL) ON CONFLICT(service_id) DO UPDATE SET generation=excluded.generation,checked_at=excluded.checked_at,error=NULL",params![key,generation,now()])?;
                },
                Err(error)=> {tx.execute("INSERT INTO manager_reconciliations VALUES (?1,?2,?3,?4) ON CONFLICT(service_id) DO UPDATE SET generation=excluded.generation,checked_at=excluded.checked_at,error=excluded.error",params![key,generation,now(),error.2])?;}
            }
            tx.commit()?;Ok(())
        }).await?;
    }
    state.db.call(|db| {
        db.execute("UPDATE media_files SET ownership=CASE
          WHEN (SELECT COUNT(*) FROM manager_bindings b JOIN manager_services s ON s.id=b.service_id JOIN manager_reconciliations r ON r.service_id=s.id WHERE b.file_id=media_files.id AND b.generation=media_files.generation AND b.service_generation=s.generation AND r.generation=s.generation AND r.error IS NULL AND b.checked_at>=?1)>1 THEN 'ambiguous'
          WHEN EXISTS(SELECT 1 FROM manager_services s LEFT JOIN manager_reconciliations r ON r.service_id=s.id JOIN library_roots l ON l.id=media_files.root_id WHERE s.enabled=1 AND s.kind=CASE l.kind WHEN 'movies' THEN 'radarr' WHEN 'shows' THEN 'sonarr' ELSE 'lidarr' END AND (r.error IS NOT NULL OR r.checked_at IS NULL OR r.checked_at<?1 OR r.generation<>s.generation)) THEN 'unresolved'
          WHEN EXISTS(SELECT 1 FROM manager_bindings b JOIN manager_services s ON s.id=b.service_id WHERE b.file_id=media_files.id AND b.generation=media_files.generation AND b.service_generation=s.generation AND b.checked_at>=?1 AND s.enabled=1) THEN 'managed'
          WHEN EXISTS(SELECT 1 FROM manager_bindings b WHERE b.file_id=media_files.id) THEN 'unresolved'
          ELSE 'unmanaged' END WHERE present=1",[now()-60])?;
        Ok(())
    }).await?;
    Ok(())
}
async fn reconcile_api(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.managers.guard.lock().await;
    reconcile(&state).await?;
    Ok(Json(json!({"reconciled":true})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let rows=state.db.call(|db|Ok(db.prepare("SELECT f.id,f.path,f.generation,f.ownership,COALESCE(json_group_array(json_object('service',s.name,'entity_id',b.entity_id,'file_id',b.manager_file_id,'members',json(b.members),'checked_at',b.checked_at)) FILTER (WHERE b.service_id IS NOT NULL),'[]') FROM media_files f LEFT JOIN manager_bindings b ON b.file_id=f.id LEFT JOIN manager_services s ON s.id=b.service_id WHERE f.present=1 GROUP BY f.id ORDER BY f.path LIMIT 1000")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?,"generation":r.get::<_,String>(2)?,"ownership":r.get::<_,String>(3)?,"bindings":serde_json::from_str::<Value>(&r.get::<_,String>(4)?).unwrap_or_default()})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
