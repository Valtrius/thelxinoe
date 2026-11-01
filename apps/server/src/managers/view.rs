//! The services page's one read: every service with what is shown about it, its
//! status included, from one snapshot.

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;

use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/api/v1/admin/services", get(services))
}

async fn services(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(listing(&state).await))
}

const MANAGERS: [&str; 3] = ["radarr", "sonarr", "lidarr"];

/// Each part that could not be read is named in `errors`; the rest still shows.
pub(super) async fn listing(state: &AppState) -> Value {
    let (managers, support, containers, stack, stages, connections, updates) = tokio::join!(
        super::listing(state),
        support::listing(state),
        docker(state, "containers"),
        stack::listing(state),
        transfers::listing(state),
        connections::listing(state),
        updates::listing(state),
    );
    let mut errors = serde_json::Map::new();
    let mut read = |part: &str, result: Result<Value>| {
        result
            .map_err(|error| errors.insert(part.to_owned(), json!(error.2)))
            .unwrap_or(Value::Null)
    };
    let managers = read("managers", managers);
    let support = read("support", support);
    let containers = read("containers", containers);
    let stack = read("stack", stack);
    let stages = read("stages", stages);
    let connections = read("connections", connections);
    let updates = read("updates", updates);
    // Without the controller, an installation's runtime is unknown rather than absent.
    let provisions = if stack.is_null() {
        stack::provisions(state).await.unwrap_or_default()
    } else {
        stack["provisions"].clone()
    };
    let find = |list: &Value, field: &str, value: &Value| {
        list.as_array()
            .into_iter()
            .flatten()
            .find(|row| !value.is_null() && row[field] == *value)
            .cloned()
            .unwrap_or(Value::Null)
    };
    let items = transfers::KINDS
        .iter()
        .map(|kind| {
            let kind = json!(kind);
            let mut service = find(&stages["items"], "kind", &kind);
            if service.is_null() {
                service = json!({ "kind": kind });
            }
            let manager = MANAGERS.iter().any(|m| kind == *m);
            service["integration"] = if manager {
                find(&managers["items"], "kind", &kind)
            } else {
                find(&support["items"], "kind", &kind)
            };
            service["provision"] = find(&provisions, "kind", &kind);
            service["runtime"] = if stack.is_null() {
                if service["provision"].is_null() {
                    Value::Null
                } else {
                    json!({"id":service["provision"]["id"],"kind":kind,"status":"unavailable",
                        "existence":"unknown","running":null,"drift":null,
                        "can_recreate":false,"can_retire":false,"can_remove":false})
                }
            } else {
                find(&stack["items"], "kind", &kind)
            };
            let target = find(&updates["services"], "kind", &kind);
            service["update"] = if target.is_null() {
                Value::Null
            } else {
                json!({
                    "target": target,
                    "policy": find(&updates["policies"], "service_id", &target["id"]),
                    "latest": find(&updates["items"], "service_id", &target["id"]),
                })
            };
            let unavailable = errors.contains_key("stack")
                || errors.contains_key(if manager { "managers" } else { "support" });
            service["status"] = status(&service, unavailable);
            service
        })
        .collect::<Vec<_>>();
    json!({
        "items": items,
        "containers": containers["items"].as_array().cloned().unwrap_or_default(),
        "connections": connections["items"].as_array().cloned().unwrap_or_default(),
        "timezone": updates["timezone"],
        "server_policy": updates["server_policy"],
        "errors": errors,
    })
}

fn status(service: &Value, unavailable: bool) -> Value {
    let (label, tone) = label(service, unavailable);
    json!({ "label": label, "tone": tone })
}

fn label(service: &Value, unavailable: bool) -> (&'static str, &'static str) {
    let present = |value: &Value| value.as_str().is_some_and(|text| !text.is_empty());
    let runtime = &service["runtime"];
    let integration = &service["integration"];
    if runtime["registered"] == false {
        return ("Setup mismatch", "warn");
    }
    if service["operation"]["type"] == "transfer" {
        match service["operation"]["state"].as_str() {
            Some("queued") => return ("Transfer queued", "busy"),
            Some("running") => return ("Transferring", "busy"),
            Some("committed") => return ("Owned · finishing transfer", "busy"),
            Some("recovery") => return ("Recovering", "busy"),
            Some("failed") => return ("Transfer needs attention", "bad"),
            Some("suspended") => return ("Transfer suspended", "warn"),
            _ => {}
        }
    }
    // A Connected service is ready only after a successful read-only check.
    if service["stage"] == "connected" {
        let connection = &service["connection"];
        return if connection["verified"] == true && !present(&connection["error"]) {
            ("Connected", "ok")
        } else {
            ("Unverified", "warn")
        };
    }
    match service["provision"]["state"].as_str() {
        Some("queued" | "installing") => return ("Installing", "busy"),
        Some("connecting") => return ("Connecting API", "busy"),
        Some("retiring") => return ("Retiring", "busy"),
        Some("blocked") => return ("Setup blocked", "bad"),
        _ => {}
    }
    if service["update"]["latest"]["state"] == "activating" {
        return ("Connecting API", "busy");
    }
    match runtime["status"].as_str() {
        Some("unavailable") => return ("Status unavailable", "warn"),
        Some("missing") => return ("Container missing", "bad"),
        Some("ready") => return ("Ready", "ok"),
        _ => {}
    }
    if runtime["running"] == false {
        ("Stopped", "muted")
    } else if present(&integration["error"]) {
        ("API unavailable", "warn")
    } else if runtime["running"] == true {
        ("Running", "ok")
    } else if !integration.is_null() {
        ("Connected", "ok")
    } else if unavailable {
        ("Status unavailable", "warn")
    } else {
        ("Not connected", "muted")
    }
}
