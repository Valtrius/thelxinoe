//! Runs inside the candidate's loopback-only network namespace, without Docker access.
use serde_json::{Value, json};
use std::path::Path;

pub async fn run(kind: &str, isolated: bool) -> anyhow::Result<()> {
    let template =
        crate::templates::find(kind).ok_or_else(|| anyhow::anyhow!("Unsupported adapter"))?;
    anyhow::ensure!(
        !Path::new("/var/run/docker.sock").exists()
            && !Path::new("/run/thelxinoe/controller.sock").exists(),
        "Privileged socket exposed"
    );
    if isolated {
        let interfaces =
            std::fs::read_dir("/sys/class/net")?.collect::<std::io::Result<Vec<_>>>()?;
        anyhow::ensure!(
            interfaces.len() == 1 && interfaces[0].file_name() == "lo",
            "Candidate network is not isolated"
        );
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
    let base = format!("http://127.0.0.1:{}", template.port);
    if isolated && kind == "bazarr" {
        // Only version lookups are supported. Other peer requests fail closed so
        // background sync cannot mistake an empty fixture for the real library.
        for port in [7878, 8989] {
            let listener =
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
            let stub = axum::Router::new()
                .route(
                    "/api/v3/system/status",
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
    anyhow::ensure!(!secret.is_empty(), "Service credential unavailable");
    Ok((username, secret))
}
