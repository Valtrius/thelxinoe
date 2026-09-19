use super::*;
use axum::extract::Query;
use thelxinoe_core::Role;
pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/acquisition/services", get(services))
        .route("/api/v1/acquisition/search", get(search))
        .route("/api/v1/acquisition/requests", get(list).post(request))
        .route("/api/v1/acquisition/requests/{id}", post(decide))
        .route("/api/v1/admin/acquisition/users", get(approval_users))
        .route(
            "/api/v1/admin/acquisition/users/{id}",
            axum::routing::put(auto_approve),
        )
}
async fn services(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    let rows=state.db.call(|db|Ok(db.prepare("SELECT id,name,kind,defaults<>'{}' FROM manager_services WHERE enabled=1 ORDER BY kind,name")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"ready":r.get::<_,bool>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
pub(super) fn endpoint(kind: &str) -> &'static str {
    match kind {
        "radarr" => "movie",
        "sonarr" => "series",
        _ => "album",
    }
}
pub(super) fn external(kind: &str, row: &Value) -> Option<String> {
    match kind {
        "radarr" => row["tmdbId"].as_u64().map(|v| v.to_string()),
        "sonarr" => row["tvdbId"].as_u64().map(|v| v.to_string()),
        _ => row["foreignAlbumId"].as_str().map(str::to_owned),
    }
}
fn identifier(kind: &str, value: &str) -> bool {
    if kind == "lidarr" {
        uuid::Uuid::parse_str(value).is_ok()
    } else {
        value.len() <= 12 && value.parse::<u64>().is_ok_and(|v| v > 0)
    }
}
async fn lookup(c: &Connection<'_>, external_id: &str) -> Result<Value> {
    if !identifier(&c.kind, external_id) {
        return Err(ApiError::bad("Invalid provider identifier"));
    }
    let prefix = match c.kind.as_str() {
        "radarr" => "tmdb",
        "sonarr" => "tvdb",
        _ => "mbid",
    };
    let rows = c
        .call(
            reqwest::Method::GET,
            &format!("{}/lookup", endpoint(&c.kind)),
            &[("term", format!("{prefix}:{external_id}"))],
            None,
        )
        .await?;
    rows.as_array()
        .and_then(|a| {
            a.iter()
                .find(|r| external(&c.kind, r).as_deref() == Some(external_id))
        })
        .cloned()
        .ok_or_else(ApiError::not_found)
}
#[derive(Deserialize)]
struct Search {
    service_id: String,
    term: String,
}
async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Search>,
) -> Result<Json<Value>> {
    security::principal(&state, &headers).await?;
    if input.term.trim().len() < 2 || input.term.len() > 200 {
        return Err(ApiError::bad("Enter between 2 and 200 characters"));
    }
    let s = service(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let remote = c
        .call(
            reqwest::Method::GET,
            &format!("{}/lookup", endpoint(&s.kind)),
            &[("term", input.term.trim().into())],
            None,
        )
        .await?;
    let domain = match s.kind.as_str() {
        "radarr" => "movie",
        "sonarr" => "show",
        _ => "album",
    };
    let term = input.term.trim().to_owned();
    let kind = s.kind.clone();
    let local=state.db.call(move|db|Ok(db.prepare("SELECT id,title,year FROM media_cards WHERE kind=?1 AND instr(lower(title),lower(?2))>0 ORDER BY title LIMIT 50")?.query_map(params![domain,term],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"year":r.get::<_,Option<i64>>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    let rows=remote.as_array().ok_or_else(unavailable)?.iter().take(50).filter_map(|r|{
        let external=external(&kind,r)?;let title=r["title"].as_str()?;
        Some(json!({"external_id":external,"title":title,"year":r["year"],"overview":r["overview"].as_str().unwrap_or("").chars().take(1000).collect::<String>(),"artist":r["artist"]["artistName"]}))
    }).collect::<Vec<_>>();
    Ok(Json(json!({"service":s.name,"local":local,"items":rows})))
}
async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let admin = p.user.role == Role::Admin;
    let rows=state.db.call(move|db|Ok(db.prepare("SELECT r.id,r.title,r.state,r.created_at,r.updated_at,r.manager_id,r.error,u.username,s.name,s.kind,r.user_id FROM acquisition_requests r JOIN users u ON u.id=r.user_id JOIN manager_services s ON s.id=r.service_id WHERE ?1 OR r.user_id=?2 ORDER BY r.created_at DESC LIMIT 200")?.query_map(params![admin,p.user.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,"updated_at":r.get::<_,i64>(4)?,"manager_id":r.get::<_,Option<i64>>(5)?,"error":r.get::<_,Option<String>>(6)?,"username":r.get::<_,String>(7)?,"service":r.get::<_,String>(8)?,"kind":r.get::<_,String>(9)?,"user_id":r.get::<_,String>(10)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
#[derive(Deserialize)]
struct Request {
    service_id: String,
    external_id: String,
}
fn enqueue(tx: &rusqlite::Transaction<'_>, request: &str) -> anyhow::Result<()> {
    let key = id();
    tx.execute("INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?1,'manager.request',?2,?1,'queued',?3,?3)",params![key,json!({"request_id":request}).to_string(),now()])?;
    Ok(())
}
async fn request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let s = service(&state, &input.service_id).await?;
    let _: Defaults = serde_json::from_value(s.defaults.clone())
        .map_err(|_| ApiError::bad("Administrator must choose acquisition defaults first"))?;
    let c = Connection::open(&state, &s).await?;
    let row = lookup(&c, &input.external_id).await?;
    let title = row["title"]
        .as_str()
        .ok_or_else(unavailable)?
        .chars()
        .take(500)
        .collect::<String>();
    let result=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(existing)=tx.query_row("SELECT id,state FROM acquisition_requests WHERE user_id=?1 AND service_id=?2 AND external_id=?3 AND state NOT IN ('denied','cancelled','failed')",params![p.user.id,s.id,input.external_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?{return Ok(existing)}
        let auto=p.user.role==Role::Admin||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&p.user.id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let key=id();let status=if auto{"approved"}else{"pending"};
        tx.execute("INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?8)",params![key,p.user.id,s.id,s.generation,input.external_id,title,status,now()])?;
        if auto{enqueue(&tx,&key)?;}tx.commit()?;Ok((key,status.into()))}).await?;
    Ok(Json(json!({"id":result.0,"state":result.1})))
}
#[derive(Deserialize)]
struct Decision {
    action: String,
}
async fn decide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Decision>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if !matches!(
        input.action.as_str(),
        "approve" | "deny" | "cancel" | "reacquire"
    ) {
        return Err(ApiError::bad("Unknown request action"));
    }
    if matches!(input.action.as_str(), "approve" | "deny") && p.user.role != Role::Admin {
        return Err(ApiError::forbidden());
    }
    let changed=state.db.call(move|db|{let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT r.user_id,r.state,s.generation FROM acquisition_requests r JOIN manager_services s ON s.id=r.service_id WHERE r.id=?1",[&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?;
        let Some((owner,status,generation))=row else{return Ok(false)};
        if owner!=p.user.id&&p.user.role!=Role::Admin{return Ok(false)}
        let reacquire=input.action=="reacquire";
        if if reacquire {!matches!(status.as_str(),"requested"|"available")}else{!matches!(status.as_str(),"pending"|"failed"|"uncertain")} {return Ok(false)}
        let auto=p.user.role==Role::Admin||tx.query_row("SELECT auto_approve FROM acquisition_users WHERE user_id=?1",[&owner],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
        let next=match input.action.as_str(){"approve"=>"approved","deny"=>"denied","reacquire"=>if auto{"approved"}else{"pending"},_=>"cancelled"};
        tx.execute("UPDATE acquisition_requests SET state=?1,generation=?2,error=NULL,updated_at=?3 WHERE id=?4",params![next,generation,now(),key])?;
        if next=="approved"{enqueue(&tx,&key)?;}
        tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,?2,?3,?4)",params![p.user.id,format!("request.{}",input.action),key,now()])?;tx.commit()?;Ok(true)}).await?;
    if !changed {
        return Err(ApiError::conflict(
            "This request cannot be changed in its current state",
        ));
    }
    Ok(Json(json!({"saved":true})))
}
#[derive(Deserialize)]
struct Auto {
    enabled: bool,
}
async fn approval_users(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageUsers).await?;
    let rows=state.db.call(|db| Ok(db.prepare("SELECT u.id,u.username,COALESCE(a.auto_approve,0) FROM users u LEFT JOIN acquisition_users a ON a.user_id=u.id WHERE u.role <> 'admin' ORDER BY u.username")?.query_map([],|r| Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"enabled":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":rows})))
}
async fn auto_approve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user): Path<String>,
    Json(input): Json<Auto>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::ManageUsers).await?;
    state.db.call(move|db|{let tx=db.transaction()?;tx.execute("INSERT INTO acquisition_users VALUES (?1,?2) ON CONFLICT(user_id) DO UPDATE SET auto_approve=excluded.auto_approve",params![user,input.enabled])?;tx.execute("INSERT INTO audit(actor_id,action,target,created_at) VALUES (?1,'request.auto_approve',?2,?3)",params![p.user.id,user,now()])?;tx.commit()?;Ok(())}).await?;
    Ok(Json(json!({"saved":true})))
}
async fn update(state: &AppState, key: &str, status: &str, manager: Option<i64>) -> Result<()> {
    let key = key.to_owned();
    let status = status.to_owned();
    state.db.call(move|db|{db.execute("UPDATE acquisition_requests SET state=?1,manager_id=COALESCE(?2,manager_id),updated_at=?3 WHERE id=?4",params![status,manager,now(),key])?;Ok(())}).await?;
    Ok(())
}
pub(crate) async fn acquire(state: &AppState, job: &thelxinoe_jobs::Job) -> anyhow::Result<()> {
    let key = job.payload["request_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid acquisition job"))?
        .to_owned();
    let _guard = state.managers.guard.lock().await;
    if let Err(error) = perform(state, &key).await {
        let message = error.2;
        state.db.call(move|db|{db.execute("UPDATE acquisition_requests SET state=CASE WHEN state IN ('searching','uncertain') THEN 'uncertain' ELSE 'failed' END,error=?1,updated_at=?2 WHERE id=?3",params![message,now(),key])?;Ok(())}).await?;
        anyhow::bail!("Acquisition needs attention; review its request status")
    }
    Ok(())
}
async fn perform(state: &AppState, key: &str) -> Result<()> {
    let request_id = key.to_owned();
    let row=state.db.call(move|db|Ok(db.query_row("SELECT service_id,generation,external_id,state FROM acquisition_requests WHERE id=?1",[request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?)).await?.ok_or_else(ApiError::not_found)?;
    if matches!(
        row.3.as_str(),
        "requested" | "available" | "denied" | "cancelled"
    ) {
        return Ok(());
    }
    if row.3 == "searching" {
        return Err(ApiError::conflict(
            "The previous search may have reached the manager. Review its queue before retrying.",
        ));
    }
    if !matches!(row.3.as_str(), "approved" | "adding") {
        return Err(ApiError::conflict("Request is not approved"));
    }
    let s = service(state, &row.0).await?;
    if s.generation != row.1 {
        return Err(ApiError::conflict(
            "Manager configuration changed; approve this request again",
        ));
    }
    let defaults: Defaults = serde_json::from_value(s.defaults.clone())
        .map_err(|_| ApiError::bad("Acquisition defaults are missing"))?;
    let c = Connection::open(state, &s).await?;
    let kind = endpoint(&s.kind);
    // Re-read provider metadata and manager ownership instead of accepting a client-supplied object.
    let mut item = lookup(&c, &row.2).await?;
    let existing = c.get(kind).await?;
    let existing = existing
        .as_array()
        .ok_or_else(unavailable)?
        .iter()
        .find(|r| external(&s.kind, r).as_deref() == Some(&row.2))
        .cloned();
    update(state, key, "adding", None).await?;
    let mut item = if let Some(existing) = existing {
        existing
    } else {
        if s.kind == "lidarr" {
            let artist_id = item["artist"]["foreignArtistId"]
                .as_str()
                .ok_or_else(unavailable)?
                .to_owned();
            let artists = c.get("artist").await?;
            let existing = artists
                .as_array()
                .ok_or_else(unavailable)?
                .iter()
                .find(|a| a["foreignArtistId"] == artist_id)
                .cloned();
            let artist = if let Some(a) = existing {
                a
            } else {
                let mut artist = item["artist"].clone();
                artist["rootFolderPath"] = json!(defaults.root_folder);
                artist["qualityProfileId"] = json!(defaults.quality_profile);
                artist["metadataProfileId"] = json!(defaults.metadata_profile);
                artist["monitored"] = json!(true);
                artist["addOptions"] = json!({"monitor":"none","searchForMissingAlbums":false});
                artist
            };
            item["artistId"] = json!(artist["id"].as_i64().unwrap_or(0));
            item["artist"] = artist;
        } else {
            item["rootFolderPath"] = json!(defaults.root_folder);
            item["qualityProfileId"] = json!(defaults.quality_profile);
        }
        item["monitored"] = json!(defaults.monitored);
        if s.kind == "sonarr" {
            item["seasonFolder"] = json!(true);
            item["addOptions"] = json!({"searchForMissingEpisodes":false,"monitor":"all"});
            if let Some(seasons) = item["seasons"].as_array_mut() {
                for season in seasons {
                    season["monitored"] = json!(defaults.monitored && season["seasonNumber"] != 0);
                }
            }
        }
        if s.kind == "radarr" {
            item["minimumAvailability"] = json!("released");
            item["addOptions"] = json!({"searchForMovie":false});
        }
        c.call(reqwest::Method::POST, kind, &[], Some(item)).await?
    };
    let manager = item["id"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(unavailable)?;
    if s.kind == "lidarr" && defaults.monitored {
        let artist_id = item["artistId"]
            .as_i64()
            .or_else(|| item["artist"]["id"].as_i64())
            .filter(|v| *v > 0)
            .ok_or_else(unavailable)?;
        let mut artist = c.get(&format!("artist/{artist_id}")).await?;
        if artist["monitored"] != true {
            artist["monitored"] = json!(true);
            c.call(
                reqwest::Method::PUT,
                &format!("artist/{artist_id}"),
                &[],
                Some(artist),
            )
            .await?;
        }
    }
    if defaults.monitored && item["monitored"] != true {
        item["monitored"] = json!(true);
        c.call(
            reqwest::Method::PUT,
            &format!("{kind}/{manager}"),
            &[],
            Some(item),
        )
        .await?;
    }
    if !defaults.monitored {
        update(state, key, "requested", Some(manager)).await?;
        return Ok(());
    }
    update(state, key, "searching", Some(manager)).await?;
    let command = match s.kind.as_str() {
        "radarr" => json!({"name":"MoviesSearch","movieIds":[manager]}),
        "sonarr" => json!({"name":"SeriesSearch","seriesId":manager}),
        _ => json!({"name":"AlbumSearch","albumIds":[manager]}),
    };
    c.call(reqwest::Method::POST, "command", &[], Some(command))
        .await?;
    update(state, key, "requested", Some(manager)).await?;
    Ok(())
}
