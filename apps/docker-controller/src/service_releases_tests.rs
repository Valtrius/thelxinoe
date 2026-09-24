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
    let digest = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
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
#[ignore = "Read-only network verification against the curated public registries"]
async fn live_curated_release_metadata() {
    let client = http_client().unwrap();
    for template in crate::templates::TEMPLATES {
        let repository = registry_repository(template).unwrap();
        let token = registry_token(&client, repository).await.unwrap();
        let body = read_body(
            client
                .get(format!("https://ghcr.io/v2/{repository}/manifests/latest"))
                .bearer_auth(&token)
                .header("Accept", MANIFEST_TYPES),
        )
        .await
        .unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(&body));
        let image = format!("{}@{digest}", template.repository);
        let release = metadata(template, &image).await;
        println!("{}", json!({"kind":template.kind,"release":release}));
        assert!(
            release.version.is_some(),
            "Missing version for {}",
            template.kind
        );
        // Notes may legitimately be absent (unpublished tag or API rate limit).
    }
}
