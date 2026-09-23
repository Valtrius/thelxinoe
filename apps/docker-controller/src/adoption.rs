//! Copy an existing service into managed storage, with a durable original for recovery.
use super::*;
use reqwest::Method;

#[derive(Clone, Serialize, Deserialize)]
struct Review {
    id: String,
    kind: String,
    original: Value,
    image: String,
    deployment: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Transfer {
    review: Review,
    copied: bool,
    complete: bool,
}
fn review_path(key: &str) -> std::path::PathBuf {
    store::root()
        .join("adoption-reviews")
        .join(format!("{key}.json"))
}
fn transfer_path(key: &str) -> std::path::PathBuf {
    service_path(key).with_file_name("transfer.json")
}
pub(super) fn pending(s: &Managed) -> bool {
    transfer_path(&s.id).exists()
        && persisted::<Transfer>(store::read(&transfer_path(&s.id)))
            .is_ok_and(|transfer| !transfer.complete)
}
#[derive(Deserialize)]
pub(super) struct Preview {
    kind: String,
    container_id: String,
}
async fn inspect(d: &Deployment, input: &Preview) -> Result<(Value, String)> {
    let t = templates::find(&input.kind).ok_or_else(|| bad("Unknown curated service"))?;
    if input.container_id.len() != 64 || !input.container_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(bad("Use the full Docker container ID"));
    }
    let server = engine(&format!(
        "/containers/{}/json",
        d.server["Id"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    if t.media && policy::media_source(&server).map_err(conflict)? != d.media_source {
        return Err(conflict(
            "Server media mount changed; reconcile storage before transferring services",
        ));
    }
    let raw = engine(&format!("/containers/{}/json", input.container_id)).await?;
    let image = engine(&format!(
        "/images/{}/json",
        raw["Image"].as_str().ok_or_else(unavailable)?
    ))
    .await?;
    // Preview can describe a Compose transfer. Execution requires its explicit release acknowledgement.
    policy::validate_adoption(&raw, &image, t, &d.network, &d.media_source, true)
        .map_err(conflict)?;
    let source = mount(&raw, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?;
    if !policy::media_disjoint(source, &d.appdata_source) {
        return Err(conflict(
            "Existing appdata must be outside Thelxinoe's deployment storage",
        ));
    }
    if raw["HostConfig"]["PortBindings"]
        .as_object()
        .is_some_and(|ports| ports.keys().any(|p| p != &format!("{}/tcp", t.port)))
    {
        return Err(conflict(
            "Transfer supports only the service's standard HTTP port",
        ));
    }
    let pinned = policy::adoption_image(&raw, &image, t).map_err(conflict)?;
    Ok((raw, pinned))
}
pub(super) async fn preview(
    State(runtime): State<Runtime>,
    Json(input): Json<Preview>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    ensure_kind_available(&d, &input.kind).await?;
    let (raw, image) = inspect(&d, &input).await?;
    let review = Review {
        id: thelxinoe_core::id(),
        kind: input.kind,
        original: raw,
        image,
        deployment: d.id,
    };
    persisted(store::write_json(&review_path(&review.id), &review))?;
    Ok(Json(
        json!({"review_id":review.id,"kind":review.kind,"container_id":review.original["Id"],"name":review.original["Name"].as_str().unwrap_or("").trim_start_matches('/'),"image":review.image,"source_config":mount(&review.original,"/config")?["Source"],"managed_config":format!("{}/services/{}/appdata",d.appdata_source,review.id),"media":d.media_source,"compose_project":review.original["Config"]["Labels"]["com.docker.compose.project"],"compose_service":review.original["Config"]["Labels"]["com.docker.compose.service"],"ports":review.original["HostConfig"]["PortBindings"]}),
    ))
}
#[derive(Deserialize)]
pub(super) struct Adopt {
    operation_id: String,
    kind: String,
    container_id: String,
    #[serde(default)]
    released_compose: bool,
}
async fn checked(d: &Deployment, input: &Adopt) -> Result<Review> {
    id(&input.operation_id)?;
    let review: Review = persisted(store::read(&review_path(&input.operation_id)))?;
    if review.kind != input.kind
        || review.original["Id"] != input.container_id
        || review.deployment != d.id
    {
        return Err(conflict(
            "The transfer review does not belong to this service",
        ));
    }
    policy::transfer_owner(&review.original, input.released_compose).map_err(conflict)?;
    let (current, image) = inspect(
        d,
        &Preview {
            kind: input.kind.clone(),
            container_id: input.container_id.clone(),
        },
    )
    .await?;
    if policy::fingerprint(&current) != policy::fingerprint(&review.original)
        || image != review.image
        || current["Name"] != review.original["Name"]
    {
        return Err(conflict(
            "The source changed after review; review the transfer again",
        ));
    }
    Ok(Review {
        original: current,
        ..review
    })
}
pub(super) async fn check(
    State(runtime): State<Runtime>,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    ensure_kind_available(&d, &input.kind).await?;
    checked(&d, &input).await?;
    Ok(Json(json!({"accepted":true})))
}
fn frozen(original: &Value) -> Value {
    let mut value = original.clone();
    value["HostConfig"]["RestartPolicy"] = json!({"Name":"no","MaximumRetryCount":0});
    value
}
async fn original_stopped(transfer: &Transfer) -> Result<Value> {
    let raw = engine(&format!(
        "/containers/{}/json",
        transfer.review.original["Id"]
            .as_str()
            .ok_or_else(unavailable)?
    ))
    .await?;
    if raw["State"]["Running"] != false
        || raw["State"]["Paused"] == true
        || policy::fingerprint(&raw) != policy::fingerprint(&frozen(&transfer.review.original))
    {
        return Err(conflict(
            "Original service restarted or changed during transfer; stop it and restore its reviewed configuration",
        ));
    }
    Ok(raw)
}
pub(super) async fn adopt(
    State(runtime): State<Runtime>,
    Json(input): Json<Adopt>,
) -> Result<Json<Value>> {
    let _guard = runtime.0.service(&input.kind).await;
    let d = bootstrap().await?;
    let containers = ensure_kind_available(&d, &input.kind).await?;
    let review = checked(&d, &input).await?;
    if service_path(&input.operation_id).exists() {
        return Err(conflict(
            "This service already has a managed installation or interrupted transfer",
        ));
    }
    let t = templates::find(&input.kind).ok_or_else(unavailable)?;
    let key = &input.operation_id;
    let name = choose_service_name(t.kind, key, &containers, None)?;
    let source = mount(&review.original, "/config")?["Source"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let destination = format!("{}/services/{key}/appdata", d.appdata_source);
    let mut mounts = vec![json!({"Type":"bind","Source":destination,"Target":"/config"})];
    if t.media {
        mounts.push(json!({"Type":"bind","Source":d.media_source,"Target":"/media"}));
    }
    let mut aliases: Vec<String> =
        review.original["NetworkSettings"]["Networks"][&d.network]["Aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|name| *name != input.container_id && *name != &input.container_id[..12])
            .map(str::to_owned)
            .collect();
    aliases.push(
        review.original["Name"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches('/')
            .into(),
    );
    aliases.push(format!("thelxinoe-{}", t.kind));
    aliases.sort();
    aliases.dedup();
    aliases.retain(|alias| !alias.is_empty());
    let spec = json!({"Image":review.image,"Env":review.original["Config"]["Env"],"Labels":{"app.thelxinoe.managed-id":key,"app.thelxinoe.deployment":d.id,"app.thelxinoe.kind":t.kind},"HostConfig":{"Mounts":mounts,"NetworkMode":d.network,"RestartPolicy":{"Name":"unless-stopped"},"PortBindings":review.original["HostConfig"]["PortBindings"]},"NetworkingConfig":{"EndpointsConfig":{d.network.clone():{"Aliases":aliases}}}});
    let mut s = Managed {
        id: key.clone(),
        kind: input.kind,
        container: String::new(),
        name,
        image: review.image.clone(),
        phase: "adopting".into(),
        active_update: None,
        spec,
        expected: Value::Null,
        error: None,
    };
    let mut transfer = Transfer {
        review,
        copied: false,
        complete: false,
    };
    persisted(
        std::fs::create_dir_all(service_path(key).with_file_name("appdata")).map_err(Into::into),
    )?;
    persisted(store::write_json(&transfer_path(key), &transfer))?;
    save(&s)?;
    let outcome = async {
        // Disable restart before stopping so a daemon restart cannot revive the old writer.
        request(
            Method::POST,
            &format!("/containers/{}/update", input.container_id),
            Some(json!({"RestartPolicy":{"Name":"no","MaximumRetryCount":0}})),
        )
        .await?;
        updates::stop(&input.container_id).await?;
        original_stopped(&transfer).await?;
        updates::copy_state(&d, key, &source, &destination, false, "takeover").await?;
        transfer.copied = true;
        persisted(store::write_json(&transfer_path(key), &transfer))?;
        original_stopped(&transfer).await?;
        let created = request(
            Method::POST,
            &format!("/containers/create?name={}", s.name),
            Some(s.spec.clone()),
        )
        .await?;
        s.container = created["Id"].as_str().ok_or_else(unavailable)?.into();
        save(&s)?;
        let raw = engine(&format!("/containers/{}/json", s.container)).await?;
        if raw["Image"] != transfer.review.original["Image"] {
            return Err(conflict(
                "Replacement image differs from the reviewed service",
            ));
        }
        s.expected = policy::fingerprint(&raw);
        save(&s)?;
        updates::start(&s.container).await?;
        // Docker fills network identity and normalizes host defaults on first start.
        s.expected =
            policy::fingerprint(&engine(&format!("/containers/{}/json", s.container)).await?);
        s.phase = "connecting".into();
        save(&s)?;
        Ok::<(), (StatusCode, &'static str)>(())
    }
    .await;
    if let Err(error) = outcome {
        s.phase = "uncertain".into();
        s.error = Some(format!(
            "{}; the original config is preserved. Restore the original service to recover.",
            error.1
        ));
        save(&s)?;
        return Err(error);
    }
    Ok(Json(
        json!({"id":s.id,"kind":s.kind,"container_id":s.container,"port":t.port}),
    ))
}
pub(super) async fn complete(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    let mut transfer: Transfer = persisted(store::read(&transfer_path(&s.id)))?;
    if transfer.complete {
        return Ok(Json(json!({"accepted":true})));
    }
    if !matches!(s.phase.as_str(), "connecting" | "active") || !transfer.copied {
        return Err(conflict("Transfer is not ready for API acceptance"));
    }
    let raw = updates::verified(s, d).await?;
    if raw["State"]["Running"] != true {
        return Err(conflict("Managed replacement is not running"));
    }
    original_stopped(&transfer).await?;
    // Keep both records retryable across either write being interrupted.
    s.phase = "active".into();
    save(s)?;
    transfer.complete = true;
    persisted(store::write_json(&transfer_path(&s.id), &transfer))?;
    Ok(Json(json!({"accepted":true})))
}
pub(super) async fn reconcile(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    let transfer: Transfer = persisted(store::read(&transfer_path(&s.id)))?;
    if !transfer.copied {
        return Err(conflict(
            "Appdata copy was interrupted; restore the original service",
        ));
    }
    original_stopped(&transfer).await?;
    // The common reconciliation verifies labels, image, mounts, environment and network.
    let result = super::reconcile(d, s).await?;
    s.phase = if transfer.complete {
        "active"
    } else {
        "connecting"
    }
    .into();
    save(s)?;
    if result.0["running"] != true {
        updates::start(&s.container).await?;
        s.expected =
            policy::fingerprint(&engine(&format!("/containers/{}/json", s.container)).await?);
        save(s)?;
    }
    Ok(result)
}
pub(super) async fn restore(d: &Deployment, s: &mut Managed) -> Result<Json<Value>> {
    let transfer: Transfer = persisted(store::read(&transfer_path(&s.id)))?;
    if transfer.complete || s.active_update.is_some() {
        return Err(conflict(
            "Completed transfers use managed backups and update recovery",
        ));
    }
    let original = &transfer.review.original;
    let original_id = original["Id"].as_str().ok_or_else(unavailable)?;
    let current = engine(&format!("/containers/{original_id}/json")).await?;
    if policy::fingerprint(&current) != policy::fingerprint(&frozen(original))
        && policy::fingerprint(&current) != policy::fingerprint(original)
    {
        return Err(conflict(
            "Original Docker configuration changed; restore its reviewed configuration before recovery",
        ));
    }
    // A controller restart may leave the copy worker or a timed-out create behind.
    for row in engine("/containers/json?all=true")
        .await?
        .as_array()
        .ok_or_else(unavailable)?
    {
        if row["Labels"]["app.thelxinoe.deployment"] == d.id
            && (row["Labels"]["app.thelxinoe.update"] == s.id
                || row["Labels"]["app.thelxinoe.managed-id"] == s.id)
        {
            updates::remove(row["Id"].as_str().ok_or_else(unavailable)?).await?;
        }
    }
    request(
        Method::POST,
        &format!("/containers/{original_id}/update"),
        Some(json!({"RestartPolicy":original["HostConfig"]["RestartPolicy"]})),
    )
    .await?;
    if original["State"]["Running"] == true {
        updates::start(original_id).await?;
    }
    s.phase = "returned".into();
    s.error = None;
    save(s)?;
    Ok(Json(json!({"accepted":true,"container_id":original_id})))
}
pub(super) async fn cancel_unsubmitted(d: &Deployment, key: &str) -> Result<Json<Value>> {
    let review: Review = persisted(store::read(&review_path(key)))?;
    if review.deployment != d.id {
        return Err(conflict("Transfer review belongs to another deployment"));
    }
    let original_id = review.original["Id"].as_str().ok_or_else(unavailable)?;
    engine(&format!("/containers/{original_id}/json")).await?;
    // No service record means no Docker mutation was submitted. Leave the source as it is.
    Ok(Json(json!({"accepted":true,"container_id":original_id})))
}
