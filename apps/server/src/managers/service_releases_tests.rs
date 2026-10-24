// Coverage gap: browser fixtures cannot exercise registry manifests or remote failures.
// Failure modes: an index selects another architecture; labels from a different digest
// are accepted; a missing/invalid version becomes a guessed release; registry errors
// suppress an otherwise valid image update; a container build is treated as an app version.
use super::*;
use axum::{Router, extract::State, http::StatusCode, routing::get};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

fn blob(value: Value, blobs: &mut HashMap<String, String>) -> String {
    let body = value.to_string();
    let digest = sha256(body.as_bytes());
    blobs.insert(digest.clone(), body);
    digest
}

#[tokio::test]
async fn registry_metadata_follows_the_verified_linux_amd64_manifest() {
    let mut blobs = HashMap::new();
    let config = blob(
        json!({"architecture":"amd64","os":"linux","config":{"Labels":{
            "org.opencontainers.image.version":"6.4.4.10685-ls318"
        }}}),
        &mut blobs,
    );
    let manifest = blob(json!({"config":{"digest":config}}), &mut blobs);
    let index = blob(
        json!({"manifests":[
            {"platform":{"os":"linux","architecture":"arm64"},"digest":format!("sha256:{}", "0".repeat(64))},
            {"platform":{"os":"linux","architecture":"amd64"},"digest":manifest}
        ]}),
        &mut blobs,
    );
    let wrong_platform = blob(
        json!({"manifests":[
            {"platform":{"os":"linux","architecture":"arm64"},"digest":manifest}
        ]}),
        &mut blobs,
    );
    let tampered = format!("sha256:{}", "1".repeat(64));
    blobs.insert(tampered.clone(), blobs[&index].clone());
    let app = Router::new()
        .route(
            "/v2/{*path}",
            get(
                |State(blobs): State<Arc<HashMap<String, String>>>,
                 axum::extract::Path(path): axum::extract::Path<String>| async move {
                    blobs
                        .get(path.rsplit('/').next().unwrap())
                        .cloned()
                        .ok_or(StatusCode::NOT_FOUND)
                },
            ),
        )
        .with_state(Arc::new(blobs));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = Client::new();
    let labels = image_labels(&client, &base, "linuxserver/radarr", "test", &index)
        .await
        .unwrap();
    let release = from_labels("radarr", "image@digest", &labels);
    assert_eq!(release.version.as_deref(), Some("6.4.4.10685"));
    assert_eq!(release.build_version.as_deref(), Some("6.4.4.10685-ls318"));
    assert!(
        image_labels(
            &client,
            &base,
            "linuxserver/radarr",
            "test",
            &wrong_platform
        )
        .await
        .is_err()
    );
    assert!(
        image_labels(&client, &base, "linuxserver/radarr", "test", &tampered)
            .await
            .is_err()
    );
    assert!(
        image_labels(
            &client,
            &base,
            "linuxserver/radarr",
            "test",
            &format!("sha256:{}", "2".repeat(64))
        )
        .await
        .is_err()
    );
    task.abort();
}

#[test]
fn labels_distinguish_app_versions_from_container_builds_without_guessing() {
    let legacy = from_labels(
        "sonarr",
        "image",
        &json!({"build_version":"Linuxserver.io version:- 4.0.15.2941-ls300 Build-date:- 2026-09-01"}),
    );
    assert_eq!(legacy.version.as_deref(), Some("4.0.15.2941"));
    let seerr = from_labels(
        "seerr",
        "image",
        &json!({"org.opencontainers.image.version":"v3.4.1"}),
    );
    assert_eq!(seerr.version.as_deref(), Some("3.4.1"));
    for invalid in ["latest", "develop-123", "", "1.2.3/../../other", "1.2.3\n"] {
        assert!(
            from_labels(
                "seerr",
                "image",
                &json!({"org.opencontainers.image.version":invalid})
            )
            .version
            .is_none()
        );
    }
    assert!(from_labels("radarr", "image", &json!({})).version.is_none());
}

#[tokio::test]
async fn local_labels_are_used_without_a_registry_lookup() {
    let labels = json!({"org.opencontainers.image.version":"v3.4.1"});
    let release = metadata("seerr", "ghcr.io/seerr-team/seerr@sha256:0", &labels)
        .await
        .unwrap();
    assert_eq!(release.version.as_deref(), Some("3.4.1"));
    assert!(release.release_notes_url.is_none());
    assert!(
        metadata("radarr", "repository@new", &Value::Null)
            .await
            .is_none()
    );
}

#[tokio::test]
#[ignore = "Read-only network verification against the curated public registries"]
async fn live_curated_release_metadata() {
    let client = http_client().unwrap();
    for (kind, repository) in [
        ("radarr", "lscr.io/linuxserver/radarr"),
        ("seerr", "ghcr.io/seerr-team/seerr"),
    ] {
        let reference = format!("{repository}@");
        let (registry, _) = registry_reference(&reference).unwrap();
        let token = registry_token(&client, registry).await.unwrap();
        let body = read_body(
            client
                .get(format!("https://ghcr.io/v2/{registry}/manifests/latest"))
                .bearer_auth(&token)
                .header("Accept", MANIFEST_TYPES),
        )
        .await
        .unwrap();
        let image = format!("{repository}@{}", sha256(&body));
        let release = metadata(kind, &image, &Value::Null).await;
        println!("{}", json!({"kind":kind,"release":release}));
        assert!(
            release.is_some_and(|release| release.version.is_some()),
            "Missing version for {kind}"
        );
        // Notes may legitimately be absent (unpublished tag or API rate limit).
    }
}
