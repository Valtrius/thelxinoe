use super::*;
#[path = "connection_seerr.rs"]
pub(super) mod requests;

/// Inspect an endpoint, following its container if Docker recreated it.
async fn located(state: &AppState, endpoint: &Endpoint) -> Attempt<(Endpoint, Value)> {
    match docker(state, &format!("containers/{}", endpoint.container)).await {
        Err(error) if error.1 == "container_missing" => {
            let container = recreation::follow(state, &endpoint.id, &endpoint.container).await?;
            let observed = docker(state, &format!("containers/{container}")).await?;
            Ok((
                Endpoint {
                    container,
                    ..endpoint.clone()
                },
                observed,
            ))
        }
        observed => Ok((endpoint.clone(), observed?)),
    }
}
async fn open<'a>(state: &'a AppState, endpoint: &Endpoint) -> Attempt<(Endpoint, Connection<'a>)> {
    let (endpoint, observed) = located(state, endpoint).await?;
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
    let c = if matches!(endpoint.kind.as_str(), "radarr" | "sonarr" | "lidarr") {
        let manager = service(state, &endpoint.id).await?;
        let c = Connection::open(state, &manager).await?;
        let status = c.get("system/status").await?;
        if !status["appName"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(&endpoint.kind))
        {
            return Err(Failure::conflict(
                failures::identity(&endpoint.kind, &status["appName"]).2,
            ));
        }
        c
    } else {
        let support = support::load(state, &endpoint.id).await?;
        let c = support::connect(state, &support).await?;
        support::version(&c, &support.credentials).await?;
        c
    };
    Ok((endpoint, c))
}

struct PeerRoute {
    network: String,
    source_host: String,
    target: String,
    reach: Reach,
}
/// The addresses an existing record may use for the target. Servarr masks stored
/// credentials, so where a record points is the only evidence of the instance it reaches.
pub(super) struct Reach {
    names: Vec<String>,
    internal: u16,
    published: Vec<u16>,
    /// Published ports bound beyond loopback, which other containers can use.
    exposed: Vec<u16>,
}
impl Reach {
    fn of(target: &Value, internal: u16) -> Self {
        let mut names: Vec<String> = Vec::new();
        names.extend(
            target["name"]
                .as_str()
                .map(|n| n.trim_start_matches('/').to_owned()),
        );
        names.extend(
            target["id"]
                .as_str()
                .and_then(|id| id.get(..12))
                .map(str::to_owned),
        );
        // Records written in a service can use any network it shares with the target.
        for network in target["networks"].as_array().into_iter().flatten() {
            for key in ["address", "ipv6"] {
                names.extend(
                    network[key]
                        .as_str()
                        .filter(|v| !v.is_empty())
                        .map(str::to_owned),
                );
            }
            names.extend(
                network["aliases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
        }
        let ports = || {
            target["ports"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p["private"] == internal)
        };
        let public = |p: &Value| u16::try_from(p["public"].as_u64()?).ok();
        Self {
            names,
            internal,
            published: ports().filter_map(public).collect(),
            exposed: ports()
                .filter(|p| p["loopback"] != true)
                .filter_map(public)
                .collect(),
        }
    }
    /// Whether an address written in another service designates this target. A port
    /// published only on loopback still designates it, though other containers can't use it.
    pub(super) fn reaches(&self, host: &str, port: Option<u16>) -> bool {
        let host = host.trim().trim_start_matches('[').trim_end_matches(']');
        if self
            .names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
        {
            return true;
        }
        let Some(port) = port else {
            return false;
        };
        // Docker forwards a published port on the addresses it is bound to.
        (self.published.contains(&port) && host_address(host))
            // Containers sharing a network namespace, as with a VPN, use loopback.
            || (port == self.internal && loopback(host))
    }
    /// Whether the address designates this target only through a port published on
    /// loopback: one written before Thelxinoe took the target over, which no longer works.
    pub(super) fn stale(&self, host: &str, port: Option<u16>) -> bool {
        let host = host.trim().trim_start_matches('[').trim_end_matches(']');
        let Some(port) = port else {
            return false;
        };
        !self
            .names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
            && !(port == self.internal && loopback(host))
            && self.published.contains(&port)
            && !self.exposed.contains(&port)
            && host_address(host)
    }
}
fn loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}
/// Addresses another container can use for the Docker host: private, loopback,
/// link-local and CGNAT (Tailscale) ranges, Docker's host aliases and LAN-style names.
fn host_address(host: &str) -> bool {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            let [first, second, ..] = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || (first == 100 && (64..128).contains(&second))
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            ip.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
        }
        Err(_) => {
            let host = host.to_ascii_lowercase();
            !host.contains('.')
                || [".local", ".lan", ".home", ".internal", ".localdomain"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
        }
    }
}
async fn addresses(state: &AppState, source: &Endpoint, target: &Endpoint) -> Attempt<PeerRoute> {
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
            if let (Some(_), Some(target_address)) =
                (private(&network["address"]), private(&peer["address"]))
            {
                return Ok(PeerRoute {
                    network: network["id"].as_str().unwrap().to_owned(),
                    // Docker DNS resolves this name on a network shared by the
                    // caller. Prowlarr's Host allowlist also survives IP changes.
                    source_host: support::docker_host(&a)?.to_owned(),
                    target: if thelxinoe_core::service_gateway_auth(&target.kind) {
                        support::docker_host(&b)?.to_owned()
                    } else {
                        target_address
                    },
                    reach: Reach::of(&b, target.port),
                });
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
    let (source, c) = open(state, source).await?;
    let source = &source;
    if link.kind == "requests" && !link.enabled {
        return requests::disconnect(state, link, &c).await;
    }
    if !link.enabled {
        if link.kind == "subtitles" {
            return bazarr(state, link, &c, None, None).await;
        }
        return disconnect_arr(state, link, &c).await;
    }
    let target =
        target.ok_or_else(|| Failure::unavailable("Target service is no longer connected"))?;
    let (target, target_connection) = open(state, target).await?;
    let target = &target;
    let route = addresses(state, source, target).await?;
    tracing::debug!(network = %route.network, source = %source.id, target = %target.id, "Selected service peer route");
    let target_address = route.target;
    let reach = route.reach;
    if link.kind == "requests" {
        return requests::apply(
            state,
            link,
            &c,
            target,
            &target_connection,
            &target_address,
            &reach,
        )
        .await;
    }
    if link.kind != "subtitles" {
        // Before any change to either service, including NZBGet's category.
        let (path, implementation) = if link.kind == "application" {
            ("applications", target.kind.as_str())
        } else {
            ("downloadclient", "Nzbget")
        };
        refuse_unmanaged(&mut *link, &c, path, implementation, &reach, source, target).await?;
    }
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
            "prowlarrUrl":format!("http://{}:{}{}",route.source_host,source.port,c.url_base),
            "baseUrl":format!("http://{target_address}:{}{}",target.port,target_connection.url_base),"apiKey":target_connection.key}),
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
        // NZBGet disables authentication with an empty control password;
        // Arr requires either both credentials or neither.
        let username = if download.credentials.secret.is_empty() {
            ""
        } else {
            download.credentials.username.as_str()
        };
        (
            "downloadclient",
            "Nzbget".into(),
            json!({"host":target_address,"port":target.port,
            "useSsl":false,"username":username,"password":download.credentials.secret,
            "movieCategory":category,"tvCategory":category,"musicCategory":category,"category":category}),
        )
    };
    upsert(state, link, &c, path, &implementation, &values).await
}

/// The host and port an existing Servarr connection record points at.
fn record_address(record: &Value, path: &str) -> Option<(String, Option<u16>)> {
    let field = |name: &str| {
        record["fields"]
            .as_array()?
            .iter()
            .find(|f| f["name"] == name)
            .map(|f| f["value"].clone())
    };
    if path == "applications" {
        let url = reqwest::Url::parse(field("baseUrl")?.as_str()?).ok()?;
        return Some((url.host_str()?.to_owned(), url.port_or_known_default()));
    }
    let port = field("port")
        .and_then(|p| p.as_u64().or_else(|| p.as_str()?.parse().ok()))
        .and_then(|p| u16::try_from(p).ok());
    Some((field("host")?.as_str()?.to_owned(), port))
}

/// Never silently adopt or duplicate a connection the administrator already made to
/// this target, however it is addressed, including records from older auto-wiring.
async fn refuse_unmanaged(
    link: &mut Link,
    c: &Connection<'_>,
    path: &str,
    implementation: &str,
    reach: &Reach,
    source: &Endpoint,
    target: &Endpoint,
) -> Attempt<()> {
    let existing = c.get(path).await?;
    let rows = array(&existing)?;
    // Thelxinoe's own record, or one the upsert reports as removed.
    if let Some(id) = link.upstream_id
        && (rows.iter().any(|r| r["id"] == id) || !link.recreate_missing)
    {
        return Ok(());
    }
    if rows.iter().any(|r| r["name"] == marker(link)) {
        return Ok(());
    }
    let found = rows
        .iter()
        .filter(|r| {
            r["implementation"]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(implementation))
        })
        .find_map(|r| {
            let (host, port) = record_address(r, path)?;
            reach.reaches(&host, port).then_some((r, host, port))
        });
    if let Some((record, host, port)) = found {
        let address = port.map_or(host.clone(), |port| format!("{host}:{port}"));
        link.candidate = Some(
            json!({"id":record["id"],"name":record["name"],"address":address,"hash":digest(&fields(record))}),
        );
        let name = record["name"].as_str().unwrap_or("unnamed");
        return Err(Failure::conflict(if reach.stale(&host, port) {
            format!(
                "{} connects to {} as \"{name}\" at {address}, which no longer reaches it; import it to point it at {}",
                source.name, target.name, target.name
            )
        } else {
            format!(
                "{} already connects to {} as \"{name}\" ({address}); review it in {} before adding another",
                source.name, target.name, source.name
            )
        }));
    }
    Ok(())
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
        // `refuse_unmanaged` ran before any change to either service.
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
    let credential_revision = (path == "downloadclient")
        .then(|| digest(&json!([values["username"], values["password"]])));
    let rotate_credentials =
        credential_revision.is_some() && credential_revision != link.credential_revision;
    for field in record["fields"]
        .as_array_mut()
        .ok_or_else(|| Failure::unavailable("Invalid connection fields"))?
    {
        if let Some(value) = field["name"].as_str().and_then(|name| values.get(name))
            && (!matches!(field["name"].as_str(), Some("apiKey" | "password"))
                || link.upstream_id.is_none()
                || rotate_credentials)
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
    if link.upstream_id.is_some() && current == wanted && !rotate_credentials {
        c.call(reqwest::Method::POST, &format!("{path}/test"), &[], Some(record)).await
            .map_err(|_|Failure::unavailable("The saved connection failed its service test; review its settings and credentials in the service"))?;
        link.applied_hash = Some(wanted);
        link.pending_hash = None;
        link.prepared = false;
        link.credential_revision = credential_revision;
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
    link.credential_revision = credential_revision;
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
        "apikey":section["apikey"].as_str().unwrap_or(""),"ssl":section["ssl"].as_bool().unwrap_or(false),
        "base_url":section["base_url"].as_str().unwrap_or("").trim_end_matches('/')})
}
async fn bazarr(
    state: &AppState,
    link: &mut Link,
    c: &Connection<'_>,
    target: Option<&Endpoint>,
    target_connection: Option<(&Connection<'_>, String)>,
) -> Attempt<()> {
    // Bazarr persists both manager connections through one shared settings
    // document. Serialize its read-modify-write cycle so parallel Radarr and
    // Sonarr links cannot overwrite each other's use_* flag.
    let mutex = lock(state, &format!("bazarr-config:{}", link.source));
    let _guard = mutex.lock().await;
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
        link.candidate = Some(json!({"name":format!("Bazarr {target_kind} settings"),
            "address":format!("{}:{}",current["ip"].as_str().unwrap_or_default(),current["port"]),"hash":current_hash}));
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
        wanted["base_url"] = json!(target_connection.url_base);
    }
    let hash = digest(&wanted);
    if current_hash != hash {
        // A restart can happen after checkpointing but before Bazarr receives
        // the POST. Retain the unchanged settings as valid recovery evidence.
        link.applied_hash = Some(current_hash);
        link.pending_hash = Some(hash.clone());
        link.prepared = true;
        checkpoint(state, link).await?;
        let mut form = vec![(
            format!("settings-general-use_{target_kind}"),
            link.enabled.to_string(),
        )];
        if link.enabled {
            for name in ["ip", "port", "apikey", "ssl", "base_url"] {
                let value = wanted[name]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| wanted[name].to_string());
                form.push((format!("settings-{target_kind}-{name}"), value));
            }
        }
        c.authorize().await?;
        let response = state
            .managers
            .http
            .post(format!("{}{}/api/system/settings", c.base, c.url_base))
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

#[cfg(test)]
mod reach_tests {
    use super::*;
    #[test]
    fn existing_records_reach_the_target_by_any_address_of_this_host() {
        let target = json!({"id":"abcdef1234567890","name":"/nzbget",
            "ports":[{"private":6789,"public":16789},{"private":8080,"public":18080}],
            "networks":[{"address":"172.18.0.5","ipv6":"","aliases":["downloads"]},
                {"address":"10.20.0.7","aliases":null}]});
        let reach = Reach::of(&target, 6789);
        for (host, port) in [
            ("nzbget", None),
            ("NZBGet", Some(1)),
            ("abcdef123456", Some(6789)),
            ("downloads", Some(6789)),
            // Another network the target is on, as seen from the source.
            ("10.20.0.7", Some(6789)),
            ("192.168.1.50", Some(16789)),
            ("host.docker.internal", Some(16789)),
            ("172.17.0.1", Some(16789)),
            ("nas", Some(16789)),
            ("diskstation.local", Some(16789)),
            ("100.101.102.103", Some(16789)),
            ("[fd00::5]", Some(16789)),
            ("localhost", Some(6789)),
            ("127.0.0.1", Some(6789)),
        ] {
            assert!(reach.reaches(host, port), "{host}:{port:?}");
        }
        for (host, port) in [
            // Another instance elsewhere, even on the same port.
            ("seedbox.example.com", Some(6789)),
            ("seedbox.example.com", Some(16789)),
            ("203.0.113.9", Some(16789)),
            // This host, but not the port published for this service.
            ("192.168.1.50", Some(6789)),
            ("192.168.1.50", Some(18080)),
            ("192.168.1.50", None),
        ] {
            assert!(!reach.reaches(host, port), "{host}:{port:?}");
        }
        assert!(!reach.stale("192.168.1.50", Some(16789)));
    }
    #[test]
    fn addresses_through_a_loopback_port_designate_the_target_but_are_stale() {
        let target = json!({"id":"abcdef1234567890","name":"/thelxinoe-nzbget",
            "ports":[{"private":6789,"public":6789,"loopback":true}],
            "networks":[{"address":"172.20.0.8","aliases":["nzbget"]}]});
        let reach = Reach::of(&target, 6789);
        assert!(reach.reaches("192.168.1.200", Some(6789)));
        assert!(reach.stale("192.168.1.200", Some(6789)));
        for (host, port) in [
            ("nzbget", Some(6789)),
            ("172.20.0.8", Some(6789)),
            ("127.0.0.1", Some(6789)),
        ] {
            assert!(reach.reaches(host, port), "{host}:{port:?}");
            assert!(!reach.stale(host, port), "{host}:{port:?}");
        }
    }
    #[test]
    fn record_addresses_follow_each_record_kind() {
        let address = |value: Value, path| record_address(&value, path);
        assert_eq!(
            address(
                json!({"fields":[{"name":"baseUrl","value":"http://192.168.1.50:17878/radarr"}]}),
                "applications"
            ),
            Some(("192.168.1.50".into(), Some(17878)))
        );
        assert_eq!(
            address(
                json!({"fields":[{"name":"baseUrl","value":"https://radarr.example.com"}]}),
                "applications"
            ),
            Some(("radarr.example.com".into(), Some(443)))
        );
        assert_eq!(
            address(
                json!({"fields":[{"name":"host","value":"nas"},{"name":"port","value":16789}]}),
                "downloadclient"
            ),
            Some(("nas".into(), Some(16789)))
        );
    }
}
