//! Recover only recorded service identities; an inspection outage is never absence.
use super::*;

fn runtime_state(expected: &Value, live: &Result<Value>) -> Value {
    match live {
        Ok(raw) => {
            let drift = !expected.is_null() && policy::fingerprint(raw) != *expected;
            json!({"existence":"present","inspection_error":null,"drift":drift,
                "running":raw["State"]["Running"],"status":if drift {"drifted"}
                    else if raw["State"]["Running"] == true {"running"} else {"stopped"}})
        }
        Err((StatusCode::NOT_FOUND, _)) => json!({"existence":"missing",
            "inspection_error":null,"drift":null,"running":null,"status":"missing"}),
        Err(_) => json!({"existence":"unknown","inspection_error":"Docker inspection unavailable",
            "drift":null,"running":null,"status":"unavailable"}),
    }
}

pub(super) fn observation(s: &Managed, live: &Result<Value>) -> Value {
    let mut value = runtime_state(&s.expected, live);
    let transfer = adoption::pending(s);
    let recoverable = recoverable(s) && !transfer && value["existence"] == "missing";
    value.as_object_mut().unwrap().extend(
        json!({
            "id":s.id,"kind":s.kind,"container_id":s.container,"name":s.name,"image":s.image,
            "phase":s.phase,"error":s.error,"transfer_pending":transfer,
            "can_recreate":recoverable,"can_retire":recoverable
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    value
}

fn recoverable(s: &Managed) -> bool {
    matches!(
        s.phase.as_str(),
        "active" | "creating" | "recreating" | "changing"
    )
}

fn candidate(
    containers: &Value,
    deployment: &str,
    key: &str,
    name: &str,
    saved: &str,
) -> Result<Option<String>> {
    let rows = containers.as_array().ok_or_else(unavailable)?;
    let mut found = None;
    for row in rows {
        let owned = row["Labels"]["app.thelxinoe.deployment"] == deployment
            && row["Labels"]["app.thelxinoe.managed-id"] == key;
        let named = row["Names"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|n| n.trim_start_matches('/') == name);
        let recorded = !saved.is_empty() && row["Id"] == saved;
        if (named || recorded) && !owned {
            return Err(conflict(
                "Recorded container identity belongs to another owner",
            ));
        }
        if owned {
            if found.is_some() {
                return Err(conflict("Multiple containers claim this managed identity"));
            }
            found = Some(row["Id"].as_str().ok_or_else(unavailable)?.to_owned());
        }
    }
    Ok(found)
}

pub(super) async fn find_container(d: &Deployment, s: &Managed) -> Result<Option<String>> {
    // A successful complete listing also catches a renamed container, preventing
    // a second writer against the same appdata after a lost create response.
    let mut rows = engine("/containers/json?all=true").await?;
    // Update backups retain the same ownership labels. Exclude only originals
    // proven stopped and unchanged against their completed update journals.
    let retained = updates::retained_originals(d, s).await?;
    rows.as_array_mut().ok_or_else(unavailable)?.retain(|row| {
        !row["Id"]
            .as_str()
            .is_some_and(|id| retained.iter().any(|old| old == id))
    });
    candidate(&rows, &d.id, &s.id, &s.name, &s.container)
}

async fn validate_storage(d: &Deployment, s: &Managed) -> Result<()> {
    let directory = service_path(&s.id).with_file_name("appdata");
    let metadata = std::fs::symlink_metadata(directory).map_err(|_| {
        conflict("Preserved appdata is missing; restore a backup before recreating")
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(conflict("Preserved appdata must be a real directory"));
    }
    if templates::find(&s.kind).is_some_and(|t| t.media) {
        let server = engine(&format!(
            "/containers/{}/json",
            d.server["Name"]
                .as_str()
                .or(d.server["Id"].as_str())
                .ok_or_else(unavailable)?
                .trim_start_matches('/')
        ))
        .await?;
        if policy::media_source(&server).map_err(conflict)? != d.media_source {
            return Err(conflict(
                "Server media mounts changed; restore the recorded layout before recovery",
            ));
        }
    }
    Ok(())
}

pub(super) async fn resume_creation(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if !matches!(s.phase.as_str(), "creating" | "recreating") || adoption::pending(s) {
        return Err(conflict("This operation cannot resume container creation"));
    }
    validate_storage(d, s).await?;
    let existing = find_container(d, s).await?;
    let container = if let Some(container) = existing {
        container
    } else {
        // Creation intent and spec are already durable. Retry the exact image,
        // never regenerate configuration or credentials over retained appdata.
        request(
            reqwest::Method::POST,
            &format!("/images/create?fromImage={}", s.image),
            None,
        )
        .await?;
        request(
            reqwest::Method::POST,
            &format!("/containers/create?name={}", s.name),
            Some(s.spec.clone()),
        )
        .await?["Id"]
            .as_str()
            .ok_or_else(unavailable)?
            .to_owned()
    };
    let raw = engine(&format!("/containers/{container}/json")).await?;
    verify_recorded(d, s, &raw).await?;
    s.container = container;
    save(s)?;
    updates::start(&s.container).await?;
    let raw = engine(&format!("/containers/{}/json", s.container)).await?;
    verify_recorded(d, s, &raw).await?;
    s.expected = policy::fingerprint(&raw);
    s.phase = "active".into();
    s.error = None;
    save(s)?;
    Ok(Json(
        json!({"accepted":true,"container_id":s.container,"running":raw["State"]["Running"]}),
    ))
}

pub(super) async fn recreate(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if !recoverable(s) || adoption::pending(s) {
        return Err(conflict(
            "Finish the pending service operation before recreating",
        ));
    }
    if find_container(d, s).await?.is_some() {
        // A repeated request after a lost successful response is harmless.
        return super::reconcile(d, s).await;
    }
    validate_storage(d, s).await?;
    s.phase = "recreating".into();
    s.expected = Value::Null;
    s.error = None;
    save(s)?;
    resume_creation(d, s).await
}

pub(super) async fn retire(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    if s.phase != "retired" && (!recoverable(s) || adoption::pending(s)) {
        return Err(conflict(
            "Finish the pending service operation before retiring",
        ));
    }
    if find_container(d, s).await?.is_some() {
        return Err(conflict(
            "Only a missing container can be retired; its appdata will be retained",
        ));
    }
    // Keep a tombstone and the entire appdata directory, so retries are safe
    // after the controller commits but the server loses the response.
    s.phase = "retired".into();
    s.error = None;
    save(s)?;
    Ok(Json(
        json!({"accepted":true,"retired":true,"appdata_preserved":true}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_and_missing_are_not_configuration_drift() {
        let raw = json!({"State":{"Running":false}});
        let expected = policy::fingerprint(&raw);
        assert_eq!(
            runtime_state(&expected, &Ok(raw.clone()))["status"],
            "stopped"
        );
        let mut running = raw;
        running["State"]["Running"] = json!(true);
        assert_eq!(runtime_state(&expected, &Ok(running))["status"], "running");
        let missing = runtime_state(&expected, &Err((StatusCode::NOT_FOUND, "missing")));
        assert_eq!(missing["existence"], "missing");
        assert!(missing["drift"].is_null());
        let offline = runtime_state(&expected, &Err(unavailable()));
        assert_eq!(offline["existence"], "unknown");
        assert!(offline["running"].is_null());
        assert_eq!(
            runtime_state(&expected, &Ok(json!({"Image":"changed"})))["status"],
            "drifted"
        );
    }

    #[test]
    fn recovery_finds_renamed_containers_and_rejects_conflicting_owners() {
        let owned = json!({"Id":"replacement","Names":["/renamed"],
            "Labels":{"app.thelxinoe.deployment":"deployment","app.thelxinoe.managed-id":"service"}});
        assert_eq!(
            candidate(
                &json!([owned.clone()]),
                "deployment",
                "service",
                "recorded",
                "old"
            )
            .unwrap(),
            Some("replacement".into())
        );
        let foreign = json!({"Id":"foreign","Names":["/recorded"],"Labels":{}});
        assert!(
            candidate(
                &json!([foreign]),
                "deployment",
                "service",
                "recorded",
                "old"
            )
            .is_err()
        );
        assert!(
            candidate(
                &json!([owned.clone(), owned]),
                "deployment",
                "service",
                "recorded",
                "old"
            )
            .is_err()
        );
        assert!(candidate(&Value::Null, "deployment", "service", "recorded", "old").is_err());
        assert_eq!(
            candidate(&json!([]), "deployment", "service", "recorded", "old").unwrap(),
            None
        );
    }
}
