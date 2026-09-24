//! Quality profiles are built from the service's schema, retaining its custom-format fields.
use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new().route(
        "/api/v1/admin/managers/{id}/quality-profiles",
        get(schema).post(create),
    )
}

fn leaves(items: &Value, out: &mut Vec<Value>) {
    for item in items.as_array().into_iter().flatten() {
        if item["quality"]["id"].is_i64() {
            out.push(item.clone());
        } else {
            leaves(&item["items"], out);
        }
    }
}

fn available(schema: &Value) -> Vec<Value> {
    let mut rows = Vec::new();
    leaves(&schema["items"], &mut rows);
    rows
}

fn standard_order(schema: &Value) -> Vec<i64> {
    let mut rows = available(schema);
    rows.retain(|r| {
        matches!(r["quality"]["resolution"].as_i64(), Some(1080 | 2160))
            && !matches!(r["quality"]["name"].as_str(), Some("Raw-HD" | "BR-DISK"))
    });
    rows.sort_by_key(|r| {
        let name = r["quality"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let source = if name.contains("remux") {
            4
        } else if name.contains("bluray") {
            3
        } else if name.contains("webdl") {
            2
        } else if name.contains("webrip") {
            1
        } else {
            0
        };
        (r["quality"]["resolution"].as_i64().unwrap_or(0), source)
    });
    rows.iter()
        .filter_map(|r| r["quality"]["id"].as_i64())
        .collect()
}

fn build(
    mut schema: Value,
    name: &str,
    order: &[i64],
    cutoff: i64,
    upgrades: bool,
) -> Result<Value> {
    let name = name.trim();
    let rows = available(&schema);
    let unique: std::collections::HashSet<_> = order.iter().collect();
    if name.is_empty()
        || name.len() > 100
        || order.is_empty()
        || unique.len() != order.len()
        || !order.contains(&cutoff)
        || order
            .iter()
            .any(|id| !rows.iter().any(|r| r["quality"]["id"] == *id))
    {
        return Err(ApiError::bad(
            "Choose a name, distinct qualities in increasing order, and an enabled upgrade target",
        ));
    }
    let mut items: Vec<Value> = rows
        .iter()
        .filter(|r| !order.contains(&r["quality"]["id"].as_i64().unwrap()))
        .cloned()
        .map(|mut r| {
            r["allowed"] = json!(false);
            r
        })
        .collect();
    for id in order {
        let mut row = rows
            .iter()
            .find(|r| r["quality"]["id"] == *id)
            .unwrap()
            .clone();
        row["allowed"] = json!(true);
        items.push(row);
    }
    schema.as_object_mut().ok_or_else(unavailable)?.remove("id");
    schema["name"] = json!(name);
    schema["items"] = json!(items);
    schema["cutoff"] = json!(cutoff);
    schema["upgradeAllowed"] = json!(upgrades);
    Ok(schema)
}

async fn template(c: &Connection<'_>) -> Result<Value> {
    if !matches!(c.kind.as_str(), "radarr" | "sonarr" | "lidarr") {
        return Err(ApiError::bad(
            "Custom profiles require Radarr, Sonarr or Lidarr",
        ));
    }
    let schema = c.get("qualityprofile/schema").await?;
    // Servarr returns one profile resource, not a list of profiles.
    if !schema["items"].is_array() {
        return Err(unavailable());
    }
    Ok(schema)
}

pub(super) async fn ensure_defaults(state: &AppState, service: &Service) -> Result<()> {
    if serde_json::from_value::<Defaults>(service.defaults.clone()).is_ok() {
        return Ok(());
    }
    let c = Connection::open(state, service).await?;
    let profiles = c.get("qualityprofile").await?;
    let profile = if matches!(service.kind.as_str(), "radarr" | "sonarr") {
        if let Some(profile) = profiles
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == "Thelxinoe 1080p–2160p")
        {
            profile.clone()
        } else {
            let schema = template(&c).await?;
            let order = standard_order(&schema);
            let cutoff = *order.last().ok_or_else(|| {
                ApiError::conflict("The service offers no 1080p or 2160p qualities")
            })?;
            c.call(
                reqwest::Method::POST,
                "qualityprofile",
                &[],
                Some(build(
                    schema,
                    "Thelxinoe 1080p–2160p",
                    &order,
                    cutoff,
                    true,
                )?),
            )
            .await?
        }
    } else {
        profiles[0].clone()
    };
    let metadata = if service.kind == "lidarr" {
        c.get("metadataprofile").await?[0]["id"].as_i64()
    } else {
        None
    };
    let defaults = Defaults {
        root_folder: canonical_root(&service.kind).into(),
        quality_profile: profile["id"].as_i64().ok_or_else(unavailable)?,
        metadata_profile: metadata,
        monitored: true,
    };
    storage::initialize_defaults(&state.db, service.id.clone(), defaults).await?;
    Ok(())
}

async fn schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let service = service(&state, &id).await?;
    let c = Connection::open(&state, &service).await?;
    Ok(Json(
        json!({"qualities":available(&template(&c).await?).iter().map(|r| r["quality"].clone()).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize)]
struct Create {
    name: String,
    qualities: Vec<i64>,
    cutoff: i64,
    #[serde(default = "yes")]
    upgrades: bool,
}
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Create>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let service = service(&state, &id).await?;
    let _guard = state.managers.guard.service(&service.kind).await;
    let c = Connection::open(&state, &service).await?;
    let result = c
        .call(
            reqwest::Method::POST,
            "qualityprofile",
            &[],
            Some(build(
                template(&c).await?,
                &input.name,
                &input.qualities,
                input.cutoff,
                input.upgrades,
            )?),
        )
        .await?;
    let mut defaults: Defaults =
        serde_json::from_value(service.defaults.clone()).map_err(|_| unavailable())?;
    defaults.quality_profile = result["id"].as_i64().ok_or_else(unavailable)?;
    storage::defaults(&state.db, id, defaults, actor).await?;
    let _ = seerr::sync_managers(&state).await;
    Ok(Json(json!({"id":result["id"],"name":result["name"]})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_profile_excludes_disc_images_and_raw_captures() {
        let qualities = [
            (1, "HDTV-720p", 720),
            (2, "WEBDL-2160p", 2160),
            (3, "Raw-HD", 1080),
            (4, "BR-DISK", 1080),
            (5, "WEBDL-1080p", 1080),
            (6, "WEBRip-1080p", 1080),
            (7, "Remux-2160p", 2160),
        ];
        let schema = json!({"items":qualities.iter().map(|(id,name,resolution)|json!({"quality":{"id":id,"name":name,"resolution":resolution}})).collect::<Vec<_>>()});
        assert_eq!(standard_order(&schema), vec![6, 5, 2, 7]);
    }
    #[test]
    fn preserves_schema_fields_and_orders_enabled_qualities() {
        let schema = json!({"id":0,"minUpgradeFormatScore":1,"formatItems":[{"format":7,"score":0}],"items":[{"quality":{"id":1},"allowed":true},{"items":[{"quality":{"id":2}},{"quality":{"id":3}}]}]});
        let profile = build(schema.clone(), "Custom", &[3, 2], 2, true).unwrap();
        assert_eq!(profile["items"][0]["allowed"], false);
        assert_eq!(profile["items"][1]["quality"]["id"], 3);
        assert_eq!(profile["items"][2]["quality"]["id"], 2);
        assert_eq!(profile["formatItems"], schema["formatItems"]);
        assert!(build(schema.clone(), "Bad", &[2, 2], 2, true).is_err());
        assert!(build(schema, "Bad", &[2], 3, true).is_err());
    }
}
