use super::*;

async fn open<'a>(state: &'a AppState, endpoint: &Endpoint) -> Attempt<Connection<'a>> {
    let observed = docker(state, &format!("containers/{}", endpoint.container)).await?;
    if observed["running"] == false {
        return Err(Failure::unavailable(format!(
            "{} is stopped",
            endpoint.name
        )));
    }
    if observed["running"] != true {
        return Err(Failure::unavailable(format!(
            "{} status is unavailable",
            endpoint.name
        )));
    }
    if matches!(endpoint.kind.as_str(), "radarr" | "sonarr" | "lidarr") {
        let manager = service(state, &endpoint.id).await?;
        let c = Connection::open(state, &manager).await?;
        let status = c.get("system/status").await?;
        if !status["appName"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(&endpoint.kind))
        {
            return Err(Failure::conflict(
                "The service API identity does not match its connection",
            ));
        }
        Ok(c)
    } else {
        let support = support::load(state, &endpoint.id).await?;
        let c = support::connect(state, &support).await?;
        support::version(&c, &support.credentials).await?;
        Ok(c)
    }
}

async fn addresses(
    state: &AppState,
    source: &Endpoint,
    target: &Endpoint,
) -> Attempt<(String, String, Vec<String>)> {
    let a = docker(state, &format!("containers/{}", source.container)).await?;
    let b = docker(state, &format!("containers/{}", target.container)).await?;
    for network in a["networks"].as_array().into_iter().flatten() {
        if let Some(peer) = b["networks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|n| n["id"] == network["id"] && n["id"].is_string())
        {
            let private = |value: &Value| {
                value
                    .as_str()
                    .and_then(|v| v.parse::<std::net::Ipv4Addr>().ok())
                    .filter(|ip| ip.is_private() || ip.is_loopback())
                    .map(|ip| ip.to_string())
            };
            if let (Some(source), Some(target)) =
                (private(&network["address"]), private(&peer["address"]))
            {
                let mut names = vec![target.clone()];
                names.extend(
                    b["name"]
                        .as_str()
                        .map(|n| n.trim_start_matches('/').to_owned()),
                );
                names.extend(
                    peer["aliases"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
                return Ok((source, target, names));
            }
        }
    }
    Err(Failure::conflict(
        "The two services must share a Docker network before connecting",
    ))
}

pub(super) async fn apply(
    state: &AppState,
    link: &mut Link,
    source: &Endpoint,
    target: Option<&Endpoint>,
) -> Attempt<()> {
    let c = open(state, source).await?;
    if !link.enabled {
        if link.kind == "subtitles" {
            return bazarr(state, link, &c, None, None).await;
        }
        return disconnect_arr(state, link, &c).await;
    }
    let target =
        target.ok_or_else(|| Failure::unavailable("Target service is no longer connected"))?;
    let target_connection = open(state, target).await?;
    let (source_address, target_address, target_names) = addresses(state, source, target).await?;
    if link.kind == "subtitles" {
        return bazarr(
            state,
            link,
            &c,
            Some(target),
            Some((&target_connection, target_address)),
        )
        .await;
    }
    let (path, implementation, values) = if link.kind == "application" {
        (
            "applications",
            target.kind.clone(),
            json!({
            "prowlarrUrl":format!("http://{source_address}:{}",source.port),
            "baseUrl":format!("http://{target_address}:{}",target.port),"apiKey":target_connection.key}),
        )
    } else {
        let download = support::load(state, &target.id).await?;
        let category = format!("thelxinoe-{}-{}", source.kind, &link.id[..12]);
        ensure_category(
            state,
            target,
            &target_connection,
            &download.credentials,
            &category,
        )
        .await?;
        (
            "downloadclient",
            "Nzbget".into(),
            json!({"host":target_address,"port":target.port,
            "useSsl":false,"username":download.credentials.username,"password":download.credentials.secret,
            "movieCategory":category,"tvCategory":category,"musicCategory":category,"category":category}),
        )
    };
    upsert(
        state,
        link,
        &c,
        path,
        &implementation,
        &values,
        &target_names,
    )
    .await
}

async fn ensure_category(
    state: &AppState,
    target: &Endpoint,
    c: &Connection<'_>,
    credentials: &support::Credentials,
    name: &str,
) -> Attempt<()> {
    let mutex = lock(state, &format!("nzbget-config:{}", target.id));
    let _guard = mutex.lock().await;
    let mut config = support::rpc(c, credentials, "loadconfig", json!([])).await?;
    let rows = config
        .as_array_mut()
        .ok_or_else(|| Failure::unavailable("NZBGet configuration is unavailable"))?;
    if rows.iter().any(|r| {
        r["Name"]
            .as_str()
            .is_some_and(|n| n.starts_with("Category") && n.ends_with(".Name"))
            && r["Value"] == name
    }) {
        return Ok(());
    }
    // Allocate beyond every existing category field; never rewrite numbered
    // slots or custom category destinations belonging to the administrator.
    let slot = rows
        .iter()
        .filter_map(|r| r["Name"].as_str())
        .filter_map(|n| {
            n.strip_prefix("Category")?
                .split_once('.')?
                .0
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| Failure::conflict("NZBGet category limit reached"))?;
    rows.push(json!({"Name":format!("Category{slot}.Name"),"Value":name}));
    if support::rpc(c, credentials, "saveconfig", json!([config])).await? != true {
        return Err(Failure::unavailable(
            "NZBGet did not save the connection category",
        ));
    }
    support::rpc(c, credentials, "reload", json!([])).await?;
    let ready = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if support::version(c, credentials).await.is_ok() {
                return;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(40), ready)
        .await
        .map_err(|_| Failure::unavailable("NZBGet is restarting; the connection will retry"))?;
    Ok(())
}

fn fields(record: &Value) -> Value {
    let mut fields = serde_json::Map::new();
    for field in record["fields"].as_array().into_iter().flatten() {
        if let Some(name) = field["name"].as_str()
            && matches!(
                name,
                "prowlarrUrl"
                    | "baseUrl"
                    | "apiKey"
                    | "host"
                    | "port"
                    | "useSsl"
                    | "username"
                    | "password"
                    | "movieCategory"
                    | "tvCategory"
                    | "musicCategory"
                    | "category"
            )
        {
            // Servarr reads mask stored secrets. Compare their presence and
            // preserve the mask on updates; the provider test checks validity.
            let value = if matches!(name, "apiKey" | "password") {
                json!(field["value"].as_str().is_some_and(|v| !v.is_empty()))
            } else {
                field["value"].clone()
            };
            fields.insert(name.into(), value);
        }
    }
    json!({"implementation":record["implementation"],"enable":record["enable"],
        "syncLevel":record["syncLevel"],"fields":fields})
}
fn matches_evidence(link: &Link, hash: &str) -> bool {
    link.applied_hash.as_deref() == Some(hash) || link.pending_hash.as_deref() == Some(hash)
}
fn marker(link: &Link) -> String {
    let name = if link.kind == "application" {
        link.target_kind.as_str()
    } else {
        "NZBGet"
    };
    format!("Thelxinoe {name} ({})", &link.id[..12])
}
fn array(value: &Value) -> Attempt<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| Failure::unavailable("Service returned an invalid connection list"))
}
async fn checkpoint(state: &AppState, link: &Link) -> Attempt<()> {
    storage::save(&state.db, link.clone()).await?;
    Ok(())
}

async fn upsert(
    state: &AppState,
    link: &mut Link,
    c: &Connection<'_>,
    path: &str,
    implementation: &str,
    values: &Value,
    target_names: &[String],
) -> Attempt<()> {
    let existing = c.get(path).await?;
    let rows = array(&existing)?;
    let saved = if let Some(id) = link.upstream_id {
        if let Some(item) = rows.iter().find(|r| r["id"] == id) {
            Some(item.clone())
        } else if link.recreate_missing {
            link.upstream_id = None;
            link.applied_hash = None;
            link.pending_hash = None;
            link.prepared = false;
            None
        } else {
            return Err(Failure::conflict(
                "This connection was removed in the service; choose Retry to recreate it",
            ));
        }
    } else {
        let candidates: Vec<_> = rows.iter().filter(|r| r["name"] == marker(link)).collect();
        if candidates.len() > 1 {
            return Err(Failure::conflict(
                "Multiple upstream connections claim this identity",
            ));
        }
        if let Some(item) = candidates.first() {
            if !link.prepared || !matches_evidence(link, &digest(&fields(item))) {
                return Err(Failure::conflict(
                    "An existing connection has conflicting settings",
                ));
            }
            link.upstream_id = item["id"].as_i64();
            Some((*item).clone())
        } else {
            None
        }
    };
    let mut record = if let Some(saved) = saved {
        if !matches_evidence(link, &digest(&fields(&saved))) {
            return Err(Failure::conflict(
                "Connection settings were changed in the service; review those changes before retrying",
            ));
        }
        saved
    } else {
        // Do not silently adopt or duplicate a manually configured connection
        // to the same target, including records created by older auto-wiring.
        let target_field = if path == "applications" {
            "baseUrl"
        } else {
            "host"
        };
        if rows.iter().any(|r| {
            r["implementation"]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(implementation))
                && r["fields"].as_array().into_iter().flatten().any(|f| {
                    if f["name"] != target_field {
                        return false;
                    }
                    let Some(address) = f["value"].as_str() else {
                        return false;
                    };
                    let host = if path == "applications" {
                        reqwest::Url::parse(address)
                            .ok()
                            .and_then(|u| u.host_str().map(str::to_owned))
                    } else {
                        Some(address.to_owned())
                    };
                    host.is_some_and(|host| {
                        target_names
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(&host))
                    })
                })
        }) {
            return Err(Failure::conflict(
                "An existing connection already targets this service; review it in the service before adding another",
            ));
        }
        let schema = c.get(&format!("{path}/schema")).await?;
        let mut record = array(&schema)?
            .iter()
            .find(|r| {
                r["implementation"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case(implementation))
            })
            .cloned()
            .ok_or_else(|| {
                Failure::conflict("Service does not support the required connection type")
            })?;
        record
            .as_object_mut()
            .ok_or_else(|| Failure::unavailable("Invalid connection schema"))?
            .remove("id");
        record["name"] = json!(marker(link));
        if path == "applications" {
            record["syncLevel"] = json!("fullSync");
        } else {
            record["enable"] = json!(true);
            record["priority"] = json!(1);
        }
        record
    };
    let current = digest(&fields(&record));
    for field in record["fields"]
        .as_array_mut()
        .ok_or_else(|| Failure::unavailable("Invalid connection fields"))?
    {
        if let Some(value) = field["name"].as_str().and_then(|name| values.get(name))
            && (!matches!(field["name"].as_str(), Some("apiKey" | "password"))
                || link.upstream_id.is_none())
        {
            field["value"] = value.clone();
        }
    }
    if path == "applications" {
        record["syncLevel"] = json!("fullSync");
    } else {
        record["enable"] = json!(true);
    }
    let wanted = digest(&fields(&record));
    if link.upstream_id.is_some() && current == wanted {
        c.call(reqwest::Method::POST, &format!("{path}/test"), &[], Some(record)).await
            .map_err(|_|Failure::unavailable("The saved connection failed its service test; review its settings and credentials in the service"))?;
        link.applied_hash = Some(wanted);
        link.pending_hash = None;
        link.prepared = false;
        return Ok(());
    }
    link.pending_hash = Some(wanted.clone());
    link.prepared = true;
    checkpoint(state, link).await?;
    let (method, endpoint) = if let Some(id) = link.upstream_id {
        (reqwest::Method::PUT, format!("{path}/{id}"))
    } else {
        (reqwest::Method::POST, path.into())
    };
    let result = c.call(method, &endpoint, &[], Some(record)).await?;
    if link.upstream_id.is_none() {
        link.upstream_id = result["id"].as_i64();
    }
    // Leave the prepared intent durable when the response is incomplete. The
    // next pass finds the exact marker and fingerprint instead of duplicating it.
    if link.upstream_id.is_none() {
        return Err(Failure::unavailable(
            "Service accepted the request without a connection identity; verifying on retry",
        ));
    }
    link.applied_hash = Some(wanted);
    link.pending_hash = None;
    link.prepared = false;
    Ok(())
}

async fn disconnect_arr(state: &AppState, link: &mut Link, c: &Connection<'_>) -> Attempt<()> {
    let path = if link.kind == "application" {
        "applications"
    } else {
        "downloadclient"
    };
    let existing = c.get(path).await?;
    let found = array(&existing)?.iter().find(|r| {
        if let Some(id) = link.upstream_id {
            r["id"] == id
        } else {
            link.prepared && r["name"] == marker(link)
        }
    });
    if let Some(record) = found {
        if !matches_evidence(link, &digest(&fields(record))) {
            return Err(Failure::conflict(
                "Connection is disabled in Thelxinoe, but its upstream settings changed; remove it in the service to finish disconnecting",
            ));
        }
        link.upstream_id = record["id"].as_i64();
        checkpoint(state, link).await?;
        let id = link
            .upstream_id
            .ok_or_else(|| Failure::unavailable("Invalid connection identity"))?;
        c.call(reqwest::Method::DELETE, &format!("{path}/{id}"), &[], None)
            .await?;
    }
    link.upstream_id = None;
    link.applied_hash = None;
    link.pending_hash = None;
    link.prepared = false;
    Ok(())
}

fn bazarr_fields(settings: &Value, kind: &str) -> Value {
    let enabled = settings["general"][format!("use_{kind}")]
        .as_bool()
        .unwrap_or(false);
    let section = &settings[kind];
    let port = section["port"]
        .as_u64()
        .or_else(|| section["port"].as_str()?.parse::<u64>().ok())
        .unwrap_or(0);
    json!({"enabled":enabled,"ip":section["ip"].as_str().unwrap_or(""),"port":port,
        "apikey":section["apikey"].as_str().unwrap_or(""),"ssl":section["ssl"].as_bool().unwrap_or(false)})
}
async fn bazarr(
    state: &AppState,
    link: &mut Link,
    c: &Connection<'_>,
    target: Option<&Endpoint>,
    target_connection: Option<(&Connection<'_>, String)>,
) -> Attempt<()> {
    // The target kind is retained even when Disconnect runs with a removed target.
    let target_kind = link.target_kind.clone();
    let settings = c.get("system/settings").await?;
    let current = bazarr_fields(&settings, &target_kind);
    let current_hash = digest(&current);
    if link.applied_hash.is_some() || link.pending_hash.is_some() {
        if !matches_evidence(link, &current_hash) {
            return Err(Failure::conflict(
                "Bazarr connection settings changed; review them in Bazarr before retrying",
            ));
        }
    } else if current["enabled"] == true
        || current["apikey"]
            .as_str()
            .is_some_and(|key| !key.is_empty())
    {
        return Err(Failure::conflict(
            "Bazarr already has settings for this manager; review them in Bazarr before connecting",
        ));
    }
    let mut wanted = current.clone();
    wanted["enabled"] = json!(link.enabled);
    if let Some((target_connection, address)) = target_connection {
        wanted["ip"] = json!(address);
        wanted["port"] = json!(target.unwrap().port);
        wanted["apikey"] = json!(target_connection.key);
        wanted["ssl"] = json!(false);
    }
    let hash = digest(&wanted);
    if current_hash != hash {
        link.pending_hash = Some(hash.clone());
        link.prepared = true;
        checkpoint(state, link).await?;
        let mut form = vec![(
            format!("settings-general-use_{target_kind}"),
            link.enabled.to_string(),
        )];
        if link.enabled {
            for name in ["ip", "port", "apikey", "ssl"] {
                let value = wanted[name]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| wanted[name].to_string());
                form.push((format!("settings-{target_kind}-{name}"), value));
            }
        }
        let response = state
            .managers
            .http
            .post(format!("{}/api/system/settings", c.base))
            .header("X-API-KEY", &c.key)
            .form(&form)
            .send()
            .await
            .map_err(|_| {
                Failure::unavailable("Bazarr is unavailable; the connection will retry")
            })?;
        if !response.status().is_success() {
            return Err(Failure::unavailable(
                "Bazarr rejected the connection settings",
            ));
        }
    }
    link.applied_hash = Some(hash);
    link.pending_hash = None;
    link.prepared = false;
    Ok(())
}
