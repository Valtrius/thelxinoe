//! Runs inside the candidate's loopback-only network namespace, without Docker access.
use serde_json::{Value, json};
use std::path::Path;

/// Read-only preview validation in a disposable worker, with no Docker socket.
pub async fn recyclarr() -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    anyhow::ensure!(
        !Path::new("/var/run/docker.sock").exists()
            && !Path::new("/run/thelxinoe/controller.sock").exists(),
        "Privileged socket exposed"
    );
    let targets: Vec<Value> = serde_json::from_str(&std::env::args().nth(2).unwrap_or_default())?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut hashes = json!({});
    for target in targets {
        let service = target["service_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing target identity"))?;
        anyhow::ensure!(
            service.len() == 36 && service.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
            "Invalid target identity"
        );
        let secret = std::fs::read_to_string(format!("/runtime/{service}"))?;
        let url = target["url"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing target URL"))?;
        let mut state = json!({});
        for endpoint in [
            "qualityprofile",
            "customformat",
            "qualitydefinition",
            "config/naming",
            "config/mediamanagement",
        ] {
            let mut response = client
                .get(format!("{url}/api/v3/{endpoint}"))
                .header("X-Api-Key", &secret)
                .send()
                .await?;
            anyhow::ensure!(
                ![401, 403].contains(&response.status().as_u16()),
                "Target API authorization failed"
            );
            anyhow::ensure!(
                response.status().is_success(),
                "Target settings unavailable"
            );
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                anyhow::ensure!(
                    bytes.len() + chunk.len() <= 16 * 1024 * 1024,
                    "Target settings exceeded their limit"
                );
                bytes.extend(chunk);
            }
            let mut value: Value = serde_json::from_slice(&bytes)?;
            if let Some(rows) = value.as_array_mut() {
                rows.sort_by_key(|r| r["id"].as_i64().unwrap_or(0));
            }
            if endpoint == "qualityprofile" {
                for profile in target["profiles"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Missing guide profiles"))?
                {
                    let matching = value
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|p| p["name"] == profile["name"])
                        .collect::<Vec<_>>();
                    anyhow::ensure!(
                        matching.len() <= 1
                            && matching.first().is_none_or(|p| profile["tracked_id"]
                                .as_i64()
                                .is_some_and(|id| p["id"] == id)),
                        "Guide profile name collides with an unowned profile"
                    );
                }
            }
            state[endpoint] = value;
        }
        hashes[service] = json!(format!(
            "{:x}",
            Sha256::digest(state.to_string().as_bytes())
        ));
    }
    println!("{hashes}");
    Ok(())
}

pub async fn recyclarr_seed() -> anyhow::Result<()> {
    let snapshots = Path::new("/fixtures/config/.thelxinoe/target-snapshots.json");
    if !snapshots.is_file() {
        return Ok(());
    }
    let snapshots: Value = serde_json::from_slice(&std::fs::read(snapshots)?)?;
    let bindings: Vec<Value> = serde_json::from_str(&std::env::args().nth(2).unwrap_or_default())?;
    let secret = std::fs::read_to_string("/fixtures/key")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut seen = std::collections::BTreeSet::new();
    for binding in bindings {
        let kind = binding["kind"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing fixture kind"))?;
        let service = binding["service_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing fixture identity"))?;
        if !seen.insert(service.to_owned()) {
            continue;
        }
        let Some(snapshot) = snapshots.get(service) else {
            continue;
        };
        anyhow::ensure!(
            matches!(kind, "radarr" | "sonarr"),
            "Unsupported fixture kind"
        );
        let port = binding["fixture_port"]
            .as_u64()
            .filter(|port| *port >= 10000 && *port < 65536)
            .ok_or_else(|| anyhow::anyhow!("Invalid fixture port"))?;
        let base = format!("http://127.0.0.1:{port}/api/v3");
        for (field, endpoint) in [
            ("sizes", "qualitydefinition/update"),
            ("naming", "config/naming"),
            ("management", "config/mediamanagement"),
        ] {
            if snapshot[field].is_null() {
                continue;
            }
            let response = client
                .put(format!("{base}/{endpoint}"))
                .header("X-Api-Key", &secret)
                .json(&snapshot[field])
                .send()
                .await?;
            anyhow::ensure!(
                response.status().is_success(),
                "Unable to seed {field} fixture"
            );
        }
        let mut ids = std::collections::BTreeMap::new();
        for format in snapshot["formats"].as_array().into_iter().flatten() {
            let mut value = format.clone();
            value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("Invalid format snapshot"))?
                .remove("id");
            let response = client
                .post(format!("{base}/customformat"))
                .header("X-Api-Key", &secret)
                .json(&value)
                .send()
                .await?;
            anyhow::ensure!(
                response.status().is_success(),
                "Unable to seed custom-format fixture"
            );
            let created: Value = response.json().await?;
            ids.insert(
                format["id"].as_i64().unwrap_or_default(),
                created["id"].clone(),
            );
        }
        let existing: Vec<Value> = client
            .get(format!("{base}/qualityprofile"))
            .header("X-Api-Key", &secret)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for profile in snapshot["profiles"].as_array().into_iter().flatten() {
            let mut value = profile.clone();
            value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("Invalid profile snapshot"))?
                .remove("id");
            if let Some(formats) = value["formatItems"].as_array_mut() {
                for format in formats {
                    if let Some(mapped) = format["format"].as_i64().and_then(|id| ids.get(&id)) {
                        format["format"] = mapped.clone();
                    }
                }
            }
            let request =
                if let Some(previous) = existing.iter().find(|p| p["name"] == profile["name"]) {
                    value["id"] = previous["id"].clone();
                    client.put(format!("{base}/qualityprofile/{}", previous["id"]))
                } else {
                    client.post(format!("{base}/qualityprofile"))
                };
            let response = request
                .header("X-Api-Key", &secret)
                .json(&value)
                .send()
                .await?;
            anyhow::ensure!(
                response.status().is_success(),
                "Unable to seed quality-profile fixture"
            );
        }
    }
    Ok(())
}

fn validate_interfaces(root: &Path) -> anyhow::Result<()> {
    let mut loopback = false;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        // Synology also exposes regular files such as syno_ovs_bonds here.
        if !std::fs::metadata(&path)?.is_dir() {
            continue;
        }
        let flags = std::fs::read_to_string(path.join("flags"))?;
        let flags = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16)?;
        if flags & libc::IFF_LOOPBACK as u32 != 0 {
            loopback = true;
        } else {
            // Kernels may create inactive fallback tunnels even with NetworkMode=none.
            anyhow::ensure!(
                flags & libc::IFF_UP as u32 == 0,
                "Candidate network is not isolated: interface {} is up",
                entry.file_name().to_string_lossy()
            );
        }
    }
    anyhow::ensure!(loopback, "Candidate loopback interface is missing");
    Ok(())
}

pub async fn run(kind: &str, isolated: bool) -> anyhow::Result<()> {
    let template =
        crate::templates::find(kind).ok_or_else(|| anyhow::anyhow!("Unsupported adapter"))?;
    anyhow::ensure!(
        !Path::new("/var/run/docker.sock").exists()
            && !Path::new("/run/thelxinoe/controller.sock").exists(),
        "Privileged socket exposed"
    );
    if isolated {
        validate_interfaces(Path::new("/sys/class/net"))?;
        for address in ["1.1.1.1:443", "169.254.169.254:80", "172.17.0.1:80"] {
            if let Ok(Ok(_)) = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                tokio::net::TcpStream::connect(address),
            )
            .await
            {
                anyhow::bail!("Candidate can reach a forbidden endpoint");
            }
        }
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    let (username, secret) = credential(kind)?;
    let url_base = if matches!(kind, "radarr" | "sonarr" | "lidarr" | "prowlarr") {
        let config = std::fs::read_to_string("/config/config.xml")?;
        config
            .split_once("<UrlBase>")
            .and_then(|(_, tail)| tail.split_once("</UrlBase>"))
            .map(|(base, _)| base.to_owned())
            .unwrap_or_default()
    } else if kind == "bazarr" {
        bazarr_peer_base("general")?
    } else {
        String::new()
    };
    anyhow::ensure!(
        url_base.is_empty() || url_base.starts_with('/') && !url_base.contains(['?', '#', '\\']),
        "Invalid service URL base"
    );
    let base = format!(
        "http://127.0.0.1:{}{}",
        std::env::var("THELXINOE_FIXTURE_PORT")
            .ok()
            .and_then(|port| port.parse::<u16>().ok())
            .or(template.port)
            .ok_or_else(|| anyhow::anyhow!("Job workloads have no HTTP endpoint"))?,
        url_base.trim_end_matches('/')
    );
    if isolated && kind == "bazarr" {
        // Only version lookups are supported. Other peer requests fail closed so
        // background sync cannot mistake an empty fixture for the real library.
        for (port, peer) in [(7878, "radarr"), (8989, "sonarr")] {
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
            let stub = axum::Router::new()
                .route(
                    &format!("{}/api/v3/system/status", bazarr_peer_base(peer)?),
                    axum::routing::get(|| async {
                        axum::Json(
                            json!({"version":"4.0.0","appName":"Isolated dependency fixture"}),
                        )
                    }),
                )
                .fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
            tokio::spawn(async move {
                let _ = axum::serve(listener, stub).await;
            });
        }
    }
    let paths: &[(&str, bool)] = match kind {
        "radarr" | "sonarr" => &[
            ("api/v3/system/status", false),
            ("api/v3/rootfolder", true),
            ("api/v3/qualityprofile", true),
            ("api/v3/downloadclient", true),
            ("api/v3/command", true),
        ],
        "lidarr" => &[
            ("api/v1/system/status", false),
            ("api/v1/rootfolder", true),
            ("api/v1/qualityprofile", true),
            ("api/v1/metadataprofile", true),
            ("api/v1/downloadclient", true),
        ],
        "prowlarr" => &[
            ("api/v1/system/status", false),
            ("api/v1/indexer", true),
            ("api/v1/applications", true),
        ],
        "bazarr" => &[
            ("api/system/tasks", false),
            ("api/system/status", false),
            ("api/movies/wanted", false),
            ("api/episodes/wanted", false),
        ],
        "seerr" => &[
            ("api/v1/settings/public", false),
            ("api/v1/auth/me", false),
            ("api/v1/settings/radarr", true),
            ("api/v1/settings/sonarr", true),
        ],
        "nzbget" => &[],
        _ => anyhow::bail!("Unsupported adapter"),
    };
    let mut ready = false;
    for _ in 0..60 {
        let request = if kind == "nzbget" {
            client
                .post(format!("{base}/jsonrpc"))
                .basic_auth(&username, Some(&secret))
                .json(&json!({"method":"version","params":[],"id":1}))
        } else {
            client
                .get(format!("{base}/{}", paths[0].0))
                .header("X-Api-Key", &secret)
        };
        if let Ok(response) = request.send().await
            && response.status().is_success()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    anyhow::ensure!(ready, "Candidate startup or authentication failed");
    for (path, array) in paths {
        let response = client
            .get(format!("{base}/{path}"))
            .header("X-Api-Key", &secret)
            .send()
            .await?;
        let value = bounded(response).await?;
        anyhow::ensure!(
            if *array {
                value.is_array()
            } else {
                value.is_object()
            },
            "Adapter response contract changed"
        );
        if path.ends_with("system/status") && kind != "bazarr" {
            anyhow::ensure!(value["version"].is_string(), "Missing service version");
        }
    }
    if kind == "nzbget" {
        for (method, array) in [("status", false), ("listgroups", true), ("history", true)] {
            let value = bounded(
                client
                    .post(format!("{base}/jsonrpc"))
                    .basic_auth(&username, Some(&secret))
                    .json(&json!({"method":method,"params":[],"id":1}))
                    .send()
                    .await?,
            )
            .await?;
            anyhow::ensure!(
                value["error"].is_null()
                    && if array {
                        value["result"].is_array()
                    } else {
                        value["result"].is_object()
                    },
                "NZBGet adapter contract changed"
            );
        }
    }
    if isolated && ["radarr", "sonarr", "lidarr"].contains(&kind) {
        dependency_contract(&client, &base, kind, &secret).await?;
    }
    Ok(())
}
async fn dependency_contract(
    client: &reqwest::Client,
    base: &str,
    kind: &str,
    secret: &str,
) -> anyhow::Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let stub=axum::Router::new().route("/jsonrpc",axum::routing::post(move |axum::Json(input):axum::Json<Value>| {
        let seen=seen.clone();async move {
            seen.fetch_add(1,Ordering::SeqCst);
            let result=match input["method"].as_str().unwrap_or("") {
                "version"=>json!("26.0"),
                "config"=>json!([{"Name":"MainDir","Value":"/media/downloads"},{"Name":"DestDir","Value":"/media/downloads/completed"},{"Name":"KeepHistory","Value":"30"},{"Name":"Category1.Name","Value":"movies"},{"Name":"Category2.Name","Value":"tv"},{"Name":"Category3.Name","Value":"music"}]),
                _=>Value::Null,
            };
            axum::Json(json!({"version":"1.1","id":input["id"],"error":if result.is_null(){json!({"code":-32601,"message":"Unsupported fixture method"})}else{Value::Null},"result":result}))
        }
    }));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, stub).await;
    });
    let outcome = async {
        let version = if kind == "lidarr" { "v1" } else { "v3" };
        let schema = bounded(
            client
                .get(format!("{base}/api/{version}/downloadclient/schema"))
                .header("X-Api-Key", secret)
                .send()
                .await?,
        )
        .await?;
        let mut settings = schema
            .as_array()
            .and_then(|a| a.iter().find(|s| s["implementation"] == "Nzbget"))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("NZBGet adapter schema unavailable"))?;
        settings["name"] = json!("Isolated contract fixture");
        settings["enable"] = json!(true);
        settings.as_object_mut().unwrap().remove("id");
        for field in settings["fields"]
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Missing adapter fields"))?
        {
            let value = match field["name"].as_str().unwrap_or("") {
                "host" => Some(json!("127.0.0.1")),
                "port" => Some(json!(port)),
                "useSsl" => Some(json!(false)),
                "username" | "password" => Some(json!("isolated-fixture")),
                "movieCategory" => Some(json!("movies")),
                "tvCategory" => Some(json!("tv")),
                "musicCategory" => Some(json!("music")),
                _ => None,
            };
            if let Some(value) = value {
                field["value"] = value;
            }
        }
        let response = client
            .post(format!("{base}/api/{version}/downloadclient/test"))
            .header("X-Api-Key", secret)
            .json(&settings)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success() && calls.load(Ordering::SeqCst) > 0,
            "Controlled dependency contract failed"
        );
        Ok(())
    }
    .await;
    task.abort();
    outcome
}
async fn bounded(mut response: reqwest::Response) -> anyhow::Result<Value> {
    anyhow::ensure!(response.status().is_success(), "Adapter endpoint failed");
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 8 * 1024 * 1024,
            "Adapter response too large"
        );
        bytes.extend(chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn credential(kind: &str) -> anyhow::Result<(String, String)> {
    let (username, secret) = if kind == "nzbget" {
        let content = std::fs::read_to_string("/config/nzbget.conf")?;
        let field = |key: &str| {
            content
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        (field("ControlUsername"), field("ControlPassword"))
    } else if kind == "seerr" {
        let config: Value = serde_json::from_slice(&std::fs::read("/config/settings.json")?)?;
        (
            String::new(),
            config["main"]["apiKey"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    } else if kind == "bazarr" {
        let content = std::fs::read_to_string("/config/config/config.yaml")?;
        let secret = content
            .lines()
            .find_map(|line| line.trim().strip_prefix("apikey:"))
            .unwrap_or("")
            .trim()
            .trim_matches(['\'', '"'])
            .to_owned();
        (String::new(), secret)
    } else {
        let content = std::fs::read_to_string("/config/config.xml")?;
        (
            String::new(),
            content
                .split_once("<ApiKey>")
                .and_then(|(_, tail)| tail.split_once("</ApiKey>"))
                .map(|(key, _)| key.to_owned())
                .unwrap_or_default(),
        )
    };
    // NZBGet without a control password accepts unauthenticated requests.
    anyhow::ensure!(
        kind == "nzbget" || !secret.is_empty(),
        "Service credential unavailable"
    );
    Ok((username, secret))
}

fn bazarr_peer_base(kind: &str) -> anyhow::Result<String> {
    let config = std::fs::read_to_string("/config/config/config.yaml")?;
    let mut section = false;
    for line in config.lines() {
        if !line.starts_with(char::is_whitespace) {
            section = line.trim() == format!("{kind}:");
        } else if section && let Some(value) = line.trim().strip_prefix("base_url:") {
            return Ok(value
                .trim()
                .trim_matches(['\'', '\"'])
                .trim_end_matches('/')
                .to_owned());
        }
    }
    Ok(String::new())
}

/// Disposable transfer validation from inside the candidate's namespace: egress sentinels
/// first, then the authenticated API and workflow contract, then preserved-record evidence.
pub async fn transfer(kind: &str, pass: &str) -> anyhow::Result<()> {
    let write = |name: &str, value: &Value| {
        let _ = std::fs::write(Path::new("/progress").join(name), value.to_string());
    };
    write("progress.json", &json!({"phase":"sentinels"}));
    let outcome = async {
        let input: Value = serde_json::from_slice(&std::fs::read("/progress/input.json")?)?;
        let sentinels = egress(&input["remotes"]).await?;
        write("progress.json", &json!({"phase":"contract"}));
        run(kind, true).await?;
        write("progress.json", &json!({"phase":"evidence"}));
        // Counts are evidence; the contract above already decided readiness.
        let evidence = evidence(kind)
            .await
            .unwrap_or_else(|error| json!({"unavailable":format!("{error:#}")}));
        Ok::<_, anyhow::Error>(json!({"pass":pass,"sentinels":sentinels,"api":evidence}))
    }
    .await;
    match &outcome {
        Ok(report) => {
            write("report.json", report);
            write("progress.json", &json!({"phase":"complete"}));
        }
        Err(error) => write(
            "progress.json",
            &json!({"phase":"failed","error":format!("{error:#}")}),
        ),
    }
    outcome.map(|_| ())
}

/// Every route out of the namespace must fail: public, DNS, metadata, host, LAN and the
/// remote services named in the copied configuration. Loopback fixtures don't count.
async fn egress(remotes: &Value) -> anyhow::Result<Value> {
    let mut targets: Vec<(String, String, u16)> = [
        ("public", "1.1.1.1", 443),
        ("public-dns", "8.8.8.8", 53),
        ("metadata", "169.254.169.254", 80),
        ("host", "172.17.0.1", 80),
        ("lan", "192.168.1.1", 80),
        ("lan", "10.0.0.1", 80),
        ("dns-name", "example.com", 443),
    ]
    .iter()
    .map(|(label, host, port)| (label.to_string(), host.to_string(), *port))
    .collect();
    for remote in remotes.as_array().into_iter().flatten().take(200) {
        if let Some(host) = remote["host"].as_str() {
            let port = remote["port"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(80);
            targets.push(("configured".into(), host.into(), port));
        }
    }
    let mut results = Vec::new();
    for (label, host, port) in targets {
        let resolved = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|addresses| {
            addresses
                .filter(|a| !a.ip().is_loopback())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        let mut reached = false;
        for address in &resolved {
            if let Ok(Ok(_)) = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                tokio::net::TcpStream::connect(address),
            )
            .await
            {
                reached = true;
            }
        }
        anyhow::ensure!(
            !reached,
            "Validation reached {label} endpoint {host}:{port}; isolation failed"
        );
        results.push(json!({"kind":label,"host":host,"port":port,"blocked":true}));
    }
    Ok(json!(results))
}

/// API-visible preserved records, read with the copied credential.
async fn evidence(kind: &str) -> anyhow::Result<Value> {
    let template =
        crate::templates::find(kind).ok_or_else(|| anyhow::anyhow!("Unsupported adapter"))?;
    let Some(port) = template.port else {
        return Ok(json!({}));
    };
    let (username, secret) = credential(kind)?;
    let base = match kind {
        "radarr" | "sonarr" | "lidarr" | "prowlarr" | "bazarr" => format!(
            "http://127.0.0.1:{port}{}",
            thelxinoe_core::service_url_base(kind)
        ),
        _ => format!("http://127.0.0.1:{port}"),
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let get = |path: String| {
        let request = client
            .get(format!("{base}/{path}"))
            .header("X-Api-Key", &secret);
        async move { bounded(request.send().await?).await }
    };
    let count = |value: Value| value.as_array().map_or(0, Vec::len);
    Ok(match kind {
        "radarr" => json!({"movies":count(get("api/v3/movie".into()).await?)}),
        "sonarr" => json!({"series":count(get("api/v3/series".into()).await?)}),
        "lidarr" => json!({"artists":count(get("api/v1/artist".into()).await?)}),
        "prowlarr" => json!({"indexers":count(get("api/v1/indexer".into()).await?),
            "applications":count(get("api/v1/applications".into()).await?)}),
        "bazarr" => {
            json!({"languages_profiles":count(get("api/system/languages/profiles".into()).await?)})
        }
        "seerr" => {
            json!({"requests":get("api/v1/request?take=1".into()).await?["pageInfo"]["results"]})
        }
        "nzbget" => {
            let history = bounded(
                client
                    .post(format!("{base}/jsonrpc"))
                    .basic_auth(&username, Some(&secret))
                    .json(&json!({"method":"history","params":[true],"id":1}))
                    .send()
                    .await?,
            )
            .await?;
            json!({"history":count(history["result"].clone())})
        }
        _ => json!({}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(interfaces: &[(&str, &str)]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for (name, flags) in interfaces {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("flags"), flags).unwrap();
        }
        root
    }

    #[test]
    fn isolated_network_accepts_loopback_and_synology_fallback_devices() {
        let root = network(&[("lo", "0x9\n")]);
        validate_interfaces(root.path()).unwrap();
        let tunnel = root.path().join("devices/sit0");
        std::fs::create_dir_all(&tunnel).unwrap();
        std::fs::write(tunnel.join("flags"), "0x80\n").unwrap();
        // Real interfaces are sysfs symlinks; Synology's bond entry is a plain file.
        let interfaces = root.path().join("net");
        std::fs::create_dir(&interfaces).unwrap();
        std::os::unix::fs::symlink(root.path().join("lo"), interfaces.join("lo")).unwrap();
        std::os::unix::fs::symlink(tunnel, interfaces.join("sit0")).unwrap();
        std::fs::write(interfaces.join("syno_ovs_bonds"), "").unwrap();
        std::fs::write(interfaces.join("bonding_masters"), "").unwrap();
        validate_interfaces(&interfaces).unwrap();
    }

    #[test]
    fn isolated_network_rejects_up_interfaces_including_tunnels() {
        for (name, flags) in [("eth0", "0x1003"), ("sit0", "0x81")] {
            let root = network(&[("lo", "0x9"), (name, flags)]);
            // operstate can be down even when the interface is administratively up.
            std::fs::write(root.path().join(name).join("operstate"), "down\n").unwrap();
            let error = validate_interfaces(root.path()).unwrap_err().to_string();
            assert_eq!(
                error,
                format!("Candidate network is not isolated: interface {name} is up")
            );
        }
    }

    #[test]
    fn isolated_network_fails_closed_when_interface_state_is_unavailable() {
        let root = network(&[("lo", "0x9"), ("sit0", "invalid")]);
        assert!(validate_interfaces(root.path()).is_err());
        std::fs::remove_file(root.path().join("sit0/flags")).unwrap();
        assert!(validate_interfaces(root.path()).is_err());
        std::fs::remove_dir(root.path().join("sit0")).unwrap();
        std::os::unix::fs::symlink("missing", root.path().join("sit0")).unwrap();
        assert!(validate_interfaces(root.path()).is_err());
        let root = network(&[("sit0", "0x80")]);
        assert!(validate_interfaces(root.path()).is_err());
    }
}
