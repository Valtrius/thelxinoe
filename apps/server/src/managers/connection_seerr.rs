use super::*;

fn evidence(record: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for key in [
        "hostname",
        "port",
        "useSsl",
        "baseUrl",
        "activeProfileId",
        "activeProfileName",
        "activeDirectory",
        "is4k",
        "isDefault",
        "syncEnabled",
        "preventSearch",
        "minimumAvailability",
        "enableSeasonFolders",
        "apiKey",
    ] {
        result.insert(key.into(), record[key].clone());
    }
    Value::Object(result)
}
pub(in crate::managers::connections) async fn verified_record(
    link: &Link,
    c: &Connection<'_>,
    kind: &str,
) -> Attempt<i64> {
    let rows = c.get(&format!("settings/{kind}")).await?;
    let id = link
        .upstream_id
        .ok_or_else(|| Failure::conflict("The managed Seerr connection is unavailable"))?;
    let record = array(&rows)?
        .iter()
        .find(|r| r["id"] == id)
        .ok_or_else(|| {
            Failure::conflict(
                "The managed Seerr connection was removed; review it in Media services",
            )
        })?;
    if !matches_evidence(link, &digest(&evidence(record))) {
        return Err(Failure::conflict(
            "Seerr connection settings changed; review them before requesting media",
        ));
    }
    Ok(id)
}
pub(super) async fn apply(
    state: &AppState,
    link: &mut Link,
    c: &Connection<'_>,
    target: &Endpoint,
    manager: &Connection<'_>,
    address: &str,
    target_names: &[String],
) -> Attempt<()> {
    if c.get("settings/public").await?["initialized"] != true {
        return Err(Failure::unavailable(
            "Finish Seerr setup before connecting its media managers",
        ));
    }
    let service = service(state, &target.id).await?;
    let defaults = serde_json::from_value::<Defaults>(service.defaults).map_err(|_| {
        Failure::conflict(
            "Choose an existing root folder and request profile for this manager first",
        )
    })?;
    let profiles = manager.get("qualityprofile").await?;
    let profile = array(&profiles)?
        .iter()
        .find(|p| p["id"] == defaults.quality_profile)
        .ok_or_else(|| Failure::conflict("The chosen request profile is unavailable"))?;
    let path = format!("settings/{}", target.kind);
    let existing = c.get(&path).await?;
    let rows = array(&existing)?;
    let name = format!("Thelxinoe {} ({})", target.kind, &link.id[..12]);
    let mut previous = link
        .upstream_id
        .and_then(|id| rows.iter().find(|r| r["id"] == id))
        .cloned();
    if previous.is_none() && link.upstream_id.is_some() {
        if !link.recreate_missing {
            return Err(Failure::conflict(
                "The Seerr connection was removed; choose Retry to recreate it",
            ));
        }
        link.upstream_id = None;
        link.applied_hash = None;
        link.pending_hash = None;
        link.prepared = false;
    }
    if previous.is_none() {
        let candidates: Vec<_> = rows.iter().filter(|r| r["name"] == name).collect();
        if candidates.len() > 1 {
            return Err(Failure::conflict(
                "Multiple Seerr connections claim this identity",
            ));
        }
        if let Some(record) = candidates.first() {
            if !link.prepared || !matches_evidence(link, &digest(&evidence(record))) {
                return Err(Failure::conflict(
                    "An existing Seerr connection has conflicting settings",
                ));
            }
            link.upstream_id = record["id"].as_i64();
            previous = Some((*record).clone());
        } else if rows.iter().any(|r| {
            r["hostname"]
                .as_str()
                .is_some_and(|h| target_names.iter().any(|n| n.eq_ignore_ascii_case(h)))
        }) {
            return Err(Failure::conflict(
                "An existing Seerr connection already targets this manager; review it before adding another",
            ));
        }
    }
    if previous
        .as_ref()
        .is_some_and(|r| !matches_evidence(link, &digest(&evidence(r))))
    {
        return Err(Failure::conflict(
            "Seerr connection settings changed; review those changes before retrying",
        ));
    }
    let revision = digest(&json!([manager.key]));
    let rotate = link.credential_revision.as_deref() != Some(&revision);
    let is4k = seerr::is_uhd(&profile["items"]);
    let is_default = previous
        .as_ref()
        .map(|r| r["isDefault"] == true)
        .unwrap_or_else(|| {
            !rows
                .iter()
                .any(|r| r["isDefault"] == true && r["is4k"] == is4k)
        });
    let mut wanted = previous
        .clone()
        .unwrap_or_else(|| json!({"name":name,"tags":[],"tagRequests":false,"overrideRule":[]}));
    let fields = json!({"hostname":address,"port":target.port,"useSsl":false,"baseUrl":manager.url_base,"activeProfileId":defaults.quality_profile,"activeProfileName":profile["name"],"activeDirectory":defaults.root_folder,"is4k":is4k,"isDefault":is_default,"syncEnabled":true,"preventSearch":!defaults.monitored,"minimumAvailability":"released","enableSeasonFolders":true});
    for (key, value) in fields.as_object().unwrap() {
        wanted[key] = value.clone();
    }
    if previous.is_none() || rotate {
        wanted["apiKey"] = json!(manager.key);
    }
    let hash = digest(&evidence(&wanted));
    if previous
        .as_ref()
        .is_some_and(|r| digest(&evidence(r)) == hash)
        && !rotate
    {
        return Ok(());
    }
    link.pending_hash = Some(hash.clone());
    link.prepared = true;
    checkpoint(state, link).await?;
    let (method, path) = if let Some(id) = link.upstream_id {
        (reqwest::Method::PUT, format!("{path}/{id}"))
    } else {
        (reqwest::Method::POST, path)
    };
    let result = c.call(method, &path, &[], Some(wanted)).await?;
    link.upstream_id = link.upstream_id.or_else(|| result["id"].as_i64());
    if link.upstream_id.is_none() {
        return Err(Failure::unavailable(
            "Seerr accepted the connection without its identity; verifying on retry",
        ));
    }
    link.applied_hash = Some(hash);
    link.pending_hash = None;
    link.prepared = false;
    link.credential_revision = Some(revision);
    checkpoint(state, link).await?;
    c.call(
        reqwest::Method::POST,
        &format!("settings/jobs/{}-scan/run", target.kind),
        &[],
        None,
    )
    .await?;
    Ok(())
}
pub(super) async fn disconnect(
    state: &AppState,
    link: &mut Link,
    c: &Connection<'_>,
) -> Attempt<()> {
    let path = format!("settings/{}", link.target_kind);
    let existing = c.get(&path).await?;
    let name = format!("Thelxinoe {} ({})", link.target_kind, &link.id[..12]);
    if let Some(record) = array(&existing)?.iter().find(|r| {
        link.upstream_id
            .map_or(link.prepared && r["name"] == name, |id| r["id"] == id)
    }) {
        if !matches_evidence(link, &digest(&evidence(record))) {
            return Err(Failure::conflict(
                "The connection is disabled in Thelxinoe, but Seerr settings changed; remove it in Seerr to finish disconnecting",
            ));
        }
        let id = record["id"]
            .as_i64()
            .ok_or_else(|| Failure::unavailable("Invalid Seerr connection identity"))?;
        link.upstream_id = Some(id);
        checkpoint(state, link).await?;
        c.call(reqwest::Method::DELETE, &format!("{path}/{id}"), &[], None)
            .await?;
    }
    link.upstream_id = None;
    link.applied_hash = None;
    link.pending_hash = None;
    link.prepared = false;
    Ok(())
}
