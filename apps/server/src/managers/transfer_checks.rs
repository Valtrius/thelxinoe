//! Reviewed post-commit setup and the completion checks of a transfer. Setup runs only the
//! actions the administrator approved, against the exact replacement, and is journaled as
//! evidence about one appdata generation: a replacement restored from the snapshot repeats it.
use super::*;

/// Setup actions a transfer may perform after its ownership commit, by service kind.
pub(super) fn setup_actions(kind: &str) -> Vec<Value> {
    match kind {
        "seerr" => vec![
            json!({"action":"seerr.managed","label":"Apply Thelxinoe's managed sign-in and Arr-based availability policy"}),
            json!({"action":"seerr.administrator","label":"Provision and read back your Thelxinoe administrator in Seerr"}),
        ],
        "prowlarr" => vec![
            json!({"action":"prowlarr.hosts","label":"Allow Thelxinoe's managed route in Prowlarr's host list"}),
        ],
        "recyclarr" => vec![
            json!({"action":"recyclarr.schedule","label":"Create a paused Thelxinoe schedule bound to Owned Radarr and Sonarr"}),
            json!({"action":"recyclarr.state","label":"Bind preserved synchronization state to those targets"}),
        ],
        _ => vec![],
    }
}

fn check(id: &str, label: &str, required: bool) -> Value {
    json!({"id":id,"label":label,"required":required,"status":"pending","detail":null,"evidence":null})
}
/// The required-check floor and the optional checks shown in review.
pub(super) fn plan(kind: &str) -> Vec<Value> {
    let mut checks = vec![
        check(
            "docker_storage",
            "Accepted Docker and storage identity",
            true,
        ),
        check("preserved_state", "Preserved application state", true),
        check("state_format", "Supported persisted-state format", true),
        check("runtime_access", "Permitted runtime and media access", true),
    ];
    if kind != "recyclarr" {
        checks.push(check("api_ready", "Authenticated API readiness", true));
    }
    let specific: &[(&str, &str, bool)] = match kind {
        "radarr" | "sonarr" | "lidarr" => &[
            ("library", "Preserved library identities", true),
            ("files", "Populated roots and files resolve", true),
            ("url_base", "Managed URL Base", true),
            ("nzbget_link", "NZBGet download client link", false),
            ("prowlarr_link", "Prowlarr application link", false),
            ("indexers", "External indexer availability", false),
        ],
        "nzbget" => &[
            ("queue_idle", "Empty queue and idle processing", true),
            ("history", "Readable preserved history and categories", true),
            ("arr_links", "Radarr, Sonarr and Lidarr client links", false),
            ("news_servers", "News-server reachability", false),
        ],
        "prowlarr" => &[
            (
                "indexers",
                "Preserved indexers, credentials and applications",
                true,
            ),
            (
                "arr_links",
                "Radarr, Sonarr and Lidarr application links",
                false,
            ),
            (
                "indexer_tests",
                "Remote indexer tests and synchronization",
                false,
            ),
        ],
        "bazarr" => &[
            (
                "profiles",
                "Preserved provider and language profiles and history",
                true,
            ),
            ("manager_links", "Radarr and Sonarr links", false),
            (
                "providers",
                "Provider reachability and live subtitle searches",
                false,
            ),
        ],
        "seerr" => &[
            ("requests", "Preserved requests and attribution", true),
            (
                "policy",
                "Managed authentication and availability policy",
                true,
            ),
            (
                "administrator",
                "Administrator identity provisioned and read back",
                true,
            ),
            ("manager_links", "Radarr and Sonarr request targets", false),
        ],
        "recyclarr" => &[
            ("configuration", "Valid imported configuration", true),
            ("targets", "Operationally ready Owned targets", true),
            (
                "target_identity",
                "Read-only target identity and profile checks",
                true,
            ),
            ("schedule", "Paused managed schedule", true),
            (
                "live_sync",
                "Live synchronization (excluded until you enable the schedule)",
                false,
            ),
        ],
        _ => &[],
    };
    checks.extend(
        specific
            .iter()
            .map(|(id, label, required)| check(id, label, *required)),
    );
    checks
}

/// Whether `action` was applied to the appdata `generation` the replacement runs now.
pub(super) fn applied<'a>(setup: &'a Value, action: &str, generation: &str) -> Option<&'a Value> {
    setup
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["action"] == action && e["state"] == "applied" && e["generation"] == generation)
}

/// Run approved setup actions not yet applied to this appdata generation. Each is recorded
/// before and after it runs.
pub(super) async fn setup(state: &AppState, t: &storage::Row, generation: &str) -> Result<()> {
    let mut journal = t.setup.as_array().cloned().unwrap_or_default();
    for action in t.review["setup_actions"].as_array().into_iter().flatten() {
        let name = action["action"].as_str().unwrap_or_default().to_owned();
        if applied(&t.setup, &name, generation).is_some() {
            continue;
        }
        journal.retain(|e| e["action"] != name.as_str());
        // An interrupted action is reconciled by repeating it: every action is idempotent.
        journal.push(
            json!({"action":name,"label":action["label"],"state":"started",
            "generation":generation,"started_at":now()}),
        );
        storage::journal(&state.db, t.id.clone(), json!(journal)).await?;
        let authority = ownership::Authority {
            service: t.service_id.clone(),
            scope: ownership::Scope::Transfer {
                operation: t.id.clone(),
                action: name.clone(),
            },
        };
        let result = ownership::with_authority(authority, apply(state, t, &name)).await;
        let entry = journal.last_mut().ok_or_else(unavailable)?;
        match &result {
            Ok(evidence) => {
                entry["state"] = json!("applied");
                entry["evidence"] = evidence.clone();
            }
            Err(error) => {
                entry["state"] = json!("failed");
                entry["error"] = json!(error.2);
            }
        }
        entry["finished_at"] = json!(now());
        storage::journal(&state.db, t.id.clone(), json!(journal)).await?;
        result?;
    }
    Ok(())
}

async fn apply(state: &AppState, t: &storage::Row, action: &str) -> Result<Value> {
    match action {
        "seerr.managed" => {
            super::seerr::initialize(state, &t.service_id).await?;
            Ok(json!({"sign_in":"Thelxinoe accounts only","availability":"Radarr and Sonarr"}))
        }
        "seerr.administrator" => {
            let approver = t.approved_by.clone().unwrap_or_else(|| t.actor_id.clone());
            let principal = storage::administrator(&state.db, approver)
                .await?
                .ok_or_else(|| {
                    ApiError::conflict("The approving administrator no longer exists")
                })?;
            let id = super::seerr::administrator(state, &t.service_id, &principal).await?;
            Ok(json!({"seerr_user":id}))
        }
        "prowlarr.hosts" => {
            super::indexers::ensure_hosts(state, &t.service_id).await?;
            Ok(json!({"applied":true}))
        }
        "recyclarr.schedule" => {
            let targets = t.review["recyclarr_targets"].clone();
            storage::recyclarr_schedule(
                &state.db,
                t.id.clone(),
                t.approved_by.clone().unwrap_or_else(|| t.actor_id.clone()),
                targets,
            )
            .await?;
            Ok(json!({"paused":true}))
        }
        "recyclarr.state" => {
            let targets = t.review["recyclarr_targets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| json!({"service_id":v["service_id"],"kind":v["kind"],"trash_id":v["trash_id"]}))
                .collect::<Vec<_>>();
            controller_call(
                state,
                &format!("/{}/recyclarr/import", t.id),
                Some(json!({"targets":targets})),
            )
            .await?;
            Ok(json!({"targets":targets.len()}))
        }
        _ => Err(ApiError::conflict(
            "The transfer review didn't approve this setup action",
        )),
    }
}

fn set(checks: &mut [Value], id: &str, status: &str, detail: impl Into<Value>, evidence: Value) {
    if let Some(check) = checks.iter_mut().find(|c| c["id"] == id) {
        check["status"] = json!(status);
        check["detail"] = detail.into();
        check["evidence"] = evidence;
    }
}
fn ids(records: &Value) -> Vec<Value> {
    records["ids"].as_array().cloned().unwrap_or_default()
}
/// Every preserved identity must be present in what the replacement reports.
fn preserved(expected: &Value, actual: &[Value]) -> (bool, usize) {
    let expected = ids(expected);
    let missing = expected.iter().filter(|id| !actual.contains(id)).count();
    (missing == 0, missing)
}

/// Evaluate the required-check floor and optional checks against the committed replacement.
pub(super) async fn evaluate(
    state: &AppState,
    t: &storage::Row,
    controller: &Value,
) -> Result<Vec<Value>> {
    let mut checks = t.review["checks"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| plan(&t.kind));
    let qualification = &controller["qualification"];
    let production = &controller["production"];
    let source = &t.review["source_state"];
    let replacement_ok = controller["replacement"].is_string() && controller["activated"] == true;
    set(
        &mut checks,
        "docker_storage",
        if replacement_ok && production["digest"].is_string() {
            "passed"
        } else {
            "failed"
        },
        Value::Null,
        json!({"replacement":controller["replacement"],"appdata_digest":production["digest"]}),
    );
    let preserved_ok = qualification["passed"] == true;
    set(
        &mut checks,
        "preserved_state",
        if preserved_ok { "passed" } else { "failed" },
        Value::Null,
        qualification["preservation"].clone(),
    );
    set(
        &mut checks,
        "state_format",
        if preserved_ok { "passed" } else { "failed" },
        Value::Null,
        json!({"versions":qualification["versions"],"formats":qualification["formats"]}),
    );
    let problems = production["access"]["problems"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    set(
        &mut checks,
        "runtime_access",
        if problems.is_empty() && production["access"].is_object() {
            "passed"
        } else {
            "failed"
        },
        json!(problems.first()),
        json!({"checked":production["access"]["checked"],"roots":production["access"]["roots"]}),
    );
    match t.kind.as_str() {
        "radarr" | "sonarr" | "lidarr" => manager_checks(state, t, source, &mut checks).await,
        "recyclarr" => recyclarr_checks(state, t, &mut checks).await,
        _ => {
            let generation = controller["production"]["generation"]
                .as_str()
                .unwrap_or_default();
            support_checks(state, t, source, generation, &mut checks).await
        }
    }
    // Optional links are completed by normal link workers once both endpoints are Owned and ready.
    let optional = optional_detail(state, &t.kind).await;
    for check in checks
        .iter_mut()
        .filter(|c| c["required"] == false && c["status"] == "pending")
    {
        let id = check["id"].as_str().unwrap_or_default().to_owned();
        check["detail"] = json!(optional.get(&id).cloned().unwrap_or_else(|| {
            "Not tested during takeover; it stays unavailable until verified after completion"
                .into()
        }));
    }
    Ok(checks)
}

async fn optional_detail(
    state: &AppState,
    kind: &str,
) -> std::collections::HashMap<String, String> {
    let owned = super::connections::owned_kinds(state)
        .await
        .unwrap_or_default();
    let peer = |kinds: &[&str], what: &str| {
        let present: Vec<&str> = kinds
            .iter()
            .copied()
            .filter(|k| owned.iter().any(|o| o == k))
            .collect();
        if present.is_empty() {
            format!(
                "No Owned {} yet; {what} stays unavailable",
                kinds.join(" or ")
            )
        } else {
            format!(
                "Connects automatically after completion with the Owned {}; it is retried independently",
                present.join(", ")
            )
        }
    };
    let mut detail = std::collections::HashMap::new();
    match kind {
        "radarr" | "sonarr" | "lidarr" => {
            detail.insert("nzbget_link".into(), peer(&["nzbget"], "downloading"));
            detail.insert(
                "prowlarr_link".into(),
                peer(&["prowlarr"], "indexer synchronization"),
            );
        }
        "nzbget" | "prowlarr" => {
            detail.insert(
                "arr_links".into(),
                peer(&["radarr", "sonarr", "lidarr"], "the link"),
            );
        }
        "bazarr" | "seerr" => {
            detail.insert(
                "manager_links".into(),
                peer(&["radarr", "sonarr"], "the link"),
            );
        }
        "recyclarr" => {
            detail.insert(
                "live_sync".into(),
                "Excluded from completion. Enable the paused schedule to synchronize".into(),
            );
        }
        _ => {}
    }
    detail
}

async fn manager_checks(state: &AppState, t: &storage::Row, source: &Value, checks: &mut [Value]) {
    let result = async {
        let s = connected(state, &t.service_id).await?;
        let c = Connection::open(state, &s).await?;
        let status = c.get("system/status").await?;
        set(
            checks,
            "api_ready",
            "passed",
            Value::Null,
            json!({"version":status["version"],"container":s.container}),
        );
        let base = thelxinoe_core::service_url_base(&t.kind);
        let reported = status["urlBase"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('/');
        set(
            checks,
            "url_base",
            if reported == base { "passed" } else { "failed" },
            json!(format!(
                "{} reports URL Base {:?}",
                failures::label(&t.kind),
                reported
            )),
            json!({"url_base":reported}),
        );
        let (path, records, files) = match t.kind.as_str() {
            "radarr" => ("movie", "movies", "movie_files"),
            "sonarr" => ("series", "series", "episode_files"),
            _ => ("artist", "artists", "track_files"),
        };
        let items = c.get(path).await?;
        let items = items.as_array().cloned().unwrap_or_default();
        let actual: Vec<Value> = items.iter().map(|i| i["id"].clone()).collect();
        let (complete, missing) = preserved(&source["records"][records], &actual);
        set(
            checks,
            "library",
            if complete { "passed" } else { "failed" },
            if complete {
                Value::Null
            } else {
                json!(format!("{missing} preserved {records} are missing"))
            },
            json!({"expected":source["records"][records]["count"],"reported":actual.len()}),
        );
        let file_count: u64 = items
            .iter()
            .map(|i| match t.kind.as_str() {
                "radarr" => u64::from(i["hasFile"] == true),
                "sonarr" => i["statistics"]["episodeFileCount"].as_u64().unwrap_or(0),
                _ => i["statistics"]["trackFileCount"].as_u64().unwrap_or(0),
            })
            .sum();
        let roots = c.get("rootfolder").await?;
        let inaccessible: Vec<Value> = roots
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r["accessible"] == false)
            .map(|r| r["path"].clone())
            .collect();
        let expected_files = source["records"][files]["count"].as_u64().unwrap_or(0);
        let files_ok = inaccessible.is_empty() && file_count >= expected_files;
        set(
            checks,
            "files",
            if files_ok { "passed" } else { "failed" },
            if files_ok {
                Value::Null
            } else if !inaccessible.is_empty() {
                json!(format!(
                    "Root folders aren't accessible: {}",
                    inaccessible
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else {
                json!(format!(
                    "{file_count} of {expected_files} imported files resolve"
                ))
            },
            json!({"files_expected":expected_files,"files_reported":file_count,"roots":roots}),
        );
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(error) = result {
        for id in ["api_ready", "library", "files", "url_base"] {
            if checks
                .iter()
                .any(|c| c["id"] == id && c["status"] == "pending")
            {
                set(checks, id, "failed", json!(error.2), Value::Null);
            }
        }
    }
}

async fn support_checks(
    state: &AppState,
    t: &storage::Row,
    source: &Value,
    generation: &str,
    checks: &mut [Value],
) {
    let result = async {
        let s = support::load_connected(state, &t.service_id).await?;
        let c = support::connect(state, &s).await?;
        let version = support::version(&c, &s.credentials).await?;
        set(checks, "api_ready", "passed", Value::Null, json!({"version":version,"container":s.container}));
        match t.kind.as_str() {
            "nzbget" => {
                let status = support::rpc(&c, &s.credentials, "status", json!([])).await?;
                let queue = support::rpc(&c, &s.credentials, "listgroups", json!([0])).await?;
                let idle = status["PostJobCount"].as_u64() == Some(0) && queue.as_array().is_some_and(Vec::is_empty);
                set(checks, "queue_idle", if idle {"passed"} else {"failed"}, if idle {Value::Null} else {json!("NZBGet has queued or post-processing downloads")},
                    json!({"queued":queue.as_array().map(Vec::len),"post_jobs":status["PostJobCount"]}));
                let history = support::rpc(&c, &s.credentials, "history", json!([true])).await?;
                let config = support::rpc(&c, &s.credentials, "config", json!([])).await?;
                let categories: Vec<Value> = config.as_array().into_iter().flatten()
                    .filter(|r| r["Name"].as_str().is_some_and(|n| n.starts_with("Category")))
                    .map(|r| r["Name"].clone()).collect();
                let (complete, missing) = preserved(&source["records"]["categories"], &categories);
                set(checks, "history", if complete && history.is_array() {"passed"} else {"failed"},
                    if complete {Value::Null} else {json!(format!("{missing} category settings are missing"))},
                    json!({"history":history.as_array().map(Vec::len),"categories":categories.len()}));
            }
            "prowlarr" => {
                let indexers: Vec<Value> = c.get("indexer").await?.as_array().into_iter().flatten().map(|i| i["id"].clone()).collect();
                let applications: Vec<Value> = c.get("applications").await?.as_array().into_iter().flatten().map(|i| i["id"].clone()).collect();
                let (indexers_ok, missing_indexers) = preserved(&source["records"]["indexers"], &indexers);
                let (applications_ok, missing_applications) = preserved(&source["records"]["applications"], &applications);
                let mut readable = true;
                for id in &indexers {
                    readable &= c.get(&format!("indexer/{id}")).await.is_ok();
                }
                let ok = indexers_ok && applications_ok && readable;
                set(checks, "indexers", if ok {"passed"} else {"failed"},
                    if ok {Value::Null} else {json!(format!("{missing_indexers} indexers and {missing_applications} applications are missing or unreadable"))},
                    json!({"indexers":indexers.len(),"applications":applications.len()}));
            }
            "bazarr" => {
                let profiles = c.get("system/languages/profiles").await?;
                let count = profiles.as_array().map_or(0, Vec::len) as u64;
                let expected = source["records"]["languages_profiles"]["count"].as_u64().unwrap_or(0);
                set(checks, "profiles", if count >= expected {"passed"} else {"failed"},
                    if count >= expected {Value::Null} else {json!(format!("{count} of {expected} language profiles are readable"))},
                    json!({"profiles":count,"history":source["records"]["history"]["count"]}));
            }
            "seerr" => {
                let requests = c.get("request?take=1").await?;
                let reported = requests["pageInfo"]["results"].as_u64().unwrap_or(0);
                let expected = source["records"]["requests"]["count"].as_u64().unwrap_or(0);
                set(checks, "requests", if reported >= expected {"passed"} else {"failed"},
                    if reported >= expected {Value::Null} else {json!(format!("{reported} of {expected} requests are readable"))},
                    json!({"requests":reported}));
                let main = c.get("settings/main").await?;
                let managed = main["localLogin"] == false && main["mediaServerLogin"] == false;
                set(checks, "policy", if managed {"passed"} else {"failed"}, Value::Null,
                    json!({"local_login":main["localLogin"],"media_server_login":main["mediaServerLogin"]}));
                let administrator = applied(&t.setup, "seerr.administrator", generation)
                    .and_then(|e| e["evidence"]["seerr_user"].as_i64());
                let read_back = match administrator {
                    Some(id) => c.get(&format!("user/{id}")).await.is_ok_and(|u| u["id"] == id),
                    None => false,
                };
                set(checks, "administrator", if read_back {"passed"} else {"failed"}, Value::Null, json!({"seerr_user":administrator}));
            }
            _ => {}
        }
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(error) = result {
        for check in checks
            .iter_mut()
            .filter(|c| c["required"] == true && c["status"] == "pending")
        {
            check["status"] = json!("failed");
            check["detail"] = json!(error.2);
        }
    }
}

async fn recyclarr_checks(state: &AppState, t: &storage::Row, checks: &mut [Value]) {
    let result = async {
        let configuration = controller_request(state, &format!("/{}/recyclarr/configuration", t.id), None).await?;
        let valid = configuration["error"].is_null();
        set(checks, "configuration", if valid {"passed"} else {"failed"}, configuration["error"].clone(), json!({"initialized":configuration["initialized"]}));
        let targets = t.review["recyclarr_targets"].as_array().cloned().unwrap_or_default();
        let mut ready = 0;
        let mut identities = Vec::new();
        for target in &targets {
            let id = target["service_id"].as_str().unwrap_or_default();
            if !ownership::operational(state, id).await? {
                continue;
            }
            ready += 1;
            let s = connected(state, id).await?;
            let c = Connection::open(state, &s).await?;
            let status = c.get("system/status").await?;
            let profiles = c.get("qualityprofile").await?;
            identities.push(json!({"service_id":id,"app":status["appName"],"profiles":profiles.as_array().map(Vec::len)}));
        }
        set(checks, "targets", if ready > 0 && ready == targets.len() {"passed"} else {"failed"},
            json!(format!("{ready} of {} targets are operational Owned services", targets.len())), json!({"targets":targets.len(),"ready":ready}));
        set(checks, "target_identity", if identities.len() == targets.len() && ready > 0 {"passed"} else {"failed"}, Value::Null, json!(identities));
        let paused = storage::recyclarr_paused(&state.db, t.id.clone()).await?;
        set(checks, "schedule", if paused {"passed"} else {"failed"}, Value::Null, json!({"paused":paused}));
        Ok::<_, ApiError>(())
    }
    .await;
    if let Err(error) = result {
        for check in checks
            .iter_mut()
            .filter(|c| c["required"] == true && c["status"] == "pending")
        {
            check["status"] = json!("failed");
            check["detail"] = json!(error.2);
        }
    }
}
