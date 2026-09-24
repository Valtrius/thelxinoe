//! Schema-driven Prowlarr onboarding. Credentials go only to the selected service.
use super::*;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/support/{id}/indexers/schema", get(schema))
        .route("/api/v1/admin/support/{id}/indexers", post(create))
        .route("/api/v1/admin/support/{id}/indexers/test", post(test))
        .route("/api/v1/admin/support/{id}/indexers/fields", post(fields))
}
pub(super) async fn ensure_hosts(state: &AppState, id: &str) -> Result<()> {
    if !installed_here(state, id).await? {
        return Ok(());
    }
    let c = connect(state, id).await?;
    let mut config = c.get("config/host").await?;
    let url = url::Url::parse(&c.base).map_err(|_| unavailable())?;
    let mut hosts = vec![
        "thelxinoe-prowlarr".to_owned(),
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        url.host_str().ok_or_else(unavailable)?.to_owned(),
    ];
    if let Some(host) = state
        .config
        .public_url
        .as_ref()
        .and_then(|url| url.host_str())
    {
        hosts.push(host.into());
    }
    if let Some(host) = storage::support_native_host(&state.db, id.to_owned())
        .await?
        .and_then(|v| url::Url::parse(&v).ok())
        .and_then(|u| u.host_str().map(str::to_owned))
    {
        hosts.push(host);
    }
    hosts.extend(
        config["allowedHosts"]
            .as_str()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty() && *host != "*")
            .map(str::to_owned),
    );
    hosts.sort();
    hosts.dedup();
    if config["allowedHosts"] == hosts.join(",") {
        return Ok(());
    }
    config["allowedHosts"] = json!(hosts.join(","));
    c.call(reqwest::Method::PUT, "config/host", &[], Some(config))
        .await?;
    Ok(())
}
async fn connect<'a>(state: &'a AppState, id: &str) -> Result<Connection<'a>> {
    let service = support::load(state, id).await?;
    let c = support::connect(state, &service).await?;
    if c.kind != "prowlarr" {
        return Err(ApiError::not_found());
    }
    Ok(c)
}
async fn schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let c = connect(&state, &id).await?;
    Ok(Json(
        json!({"items":c.get("indexer/schema").await?,"profiles":c.get("appprofile").await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    implementation: String,
    definition: String,
    name: String,
    app_profile_id: i64,
    #[serde(default)]
    fields: std::collections::BTreeMap<String, Value>,
}
fn build(schema: Value, input: Draft) -> Result<Value> {
    if input.name.trim().is_empty() || input.name.len() > 100 || input.fields.len() > 100 {
        return Err(ApiError::bad(
            "Enter an indexer name and its connection settings",
        ));
    }
    let mut body = schema
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["implementation"] == input.implementation && s["name"] == input.definition)
        .cloned()
        .ok_or_else(|| ApiError::bad("Choose an indexer from the current Prowlarr catalog"))?;
    for (name, value) in input.fields {
        if value.to_string().len() > 16_384 {
            return Err(ApiError::bad(
                "An indexer setting exceeds the supported size",
            ));
        }
        let field = body["fields"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find(|f| f["name"] == name)
            .ok_or_else(|| ApiError::bad("Unknown indexer setting"))?;
        if field["hidden"] == "hidden" && field["value"] != value {
            return Err(ApiError::bad("This indexer setting is managed by Prowlarr"));
        }
        field["value"] = value;
    }
    body.as_object_mut().ok_or_else(unavailable)?.remove("id");
    body["name"] = json!(input.name.trim());
    body["enable"] = json!(true);
    body["appProfileId"] = json!(input.app_profile_id);
    body["priority"] = json!(25);
    if body["protocol"] == "usenet" {
        body["redirect"] = json!(true);
    }
    Ok(body)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldAction {
    draft: Draft,
    field: String,
}
async fn fields(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<FieldAction>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let c = connect(&state, &id).await?;
    let body = build(c.get("indexer/schema").await?, input.draft)?;
    let field = body["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|field| field["name"] == input.field)
        .ok_or_else(|| ApiError::bad("Unknown indexer field"))?;
    let action = if field["selectOptionsProviderAction"] == "getUrls" {
        "getUrls"
    } else if field["type"] == "cardigannCaptcha" {
        "checkCaptcha"
    } else {
        return Err(ApiError::bad("This field does not provide dynamic choices"));
    };
    let result = c
        .call(
            reqwest::Method::POST,
            &format!("indexer/action/{action}"),
            &[],
            Some(body),
        )
        .await?;
    if action == "getUrls" {
        let options: Vec<Value> = result["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|option| {
                option["value"].as_str().is_some_and(|value| {
                    url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                })
            })
            .map(|option| json!({"value":option["value"],"name":option["name"]}))
            .collect();
        Ok(Json(json!({"options":options})))
    } else {
        let captcha = &result["captchaRequest"];
        if captcha["type"] == "image"
            && captcha["contentType"].as_str().is_some_and(|t| {
                matches!(t, "image/png" | "image/jpeg" | "image/gif" | "image/webp")
            })
            && captcha["imageData"]
                .as_str()
                .is_some_and(|data| data.len() <= 1_400_000)
        {
            Ok(Json(
                json!({"contentType":captcha["contentType"],"imageData":captcha["imageData"]}),
            ))
        } else {
            Err(ApiError::conflict(
                "This indexer did not return an image challenge. Check its address and credentials.",
            ))
        }
    }
}
async fn submit(
    state: AppState,
    headers: HeaderMap,
    id: String,
    input: Draft,
    test: bool,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    let _guard = state.managers.guard.service("prowlarr").await;
    let c = connect(&state, &id).await?;
    if !c
        .get("appprofile")
        .await?
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| p["id"] == input.app_profile_id)
    {
        return Err(ApiError::bad("Choose an existing sync profile"));
    }
    let body = build(c.get("indexer/schema").await?, input)?;
    let response = c
        .state
        .managers
        .http
        .post(format!(
            "{}/api/v1/{}",
            c.base,
            if test { "indexer/test" } else { "indexer" }
        ))
        .header("X-Api-Key", &c.key)
        .json(&body)
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        let errors = read(response).await.unwrap_or(Value::Null);
        let has_api_key = body["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|field| {
                field["name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("apikey"))
            });
        return Err(ApiError::conflict(validation_message(&errors, has_api_key)));
    }
    let result = read(response).await?;
    Ok(Json(
        json!({"tested":test,"id":result["id"],"name":result["name"]}),
    ))
}

fn validation_message(errors: &Value, has_api_key: bool) -> String {
    let failures: Vec<_> = errors.as_array().into_iter().flatten().collect();
    // Prowlarr reports authentication failures with an empty propertyName.
    // Recognize its messages, but never return remote response text or URLs:
    // those can contain the submitted key, password or cookie.
    let invalid_key = failures.iter().any(|error| {
        error["propertyName"]
            .as_str()
            .is_some_and(|field| field.eq_ignore_ascii_case("apikey"))
    });
    let invalid_credentials = failures.iter().any(|error| {
        let message = error["errorMessage"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        [
            "unable to connect to indexer. incorrect user credentials",
            "indexer returned result for rss url, credentials appears to be invalid.",
            "invalid api key",
            "invalid apikey",
            "api key is invalid",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix))
    });
    if invalid_key || (has_api_key && invalid_credentials) {
        return "Invalid API key. Check your indexer API key and try again.".into();
    }
    if invalid_credentials {
        return "The indexer rejected your login credentials. Check them and try again.".into();
    }
    let fields = failures
        .iter()
        .filter_map(|e| e["propertyName"].as_str())
        .filter(|v| {
            !v.is_empty()
                && v.len() < 100
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._[]".contains(c))
        })
        .collect::<Vec<_>>();
    if fields.is_empty() {
        "Prowlarr could not connect to this indexer. Check its address and connection settings, then try again.".into()
    } else {
        format!("Check these indexer settings: {}", fields.join(", "))
    }
}
async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Draft>,
) -> Result<Json<Value>> {
    submit(state, headers, id, input, false).await
}
async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Draft>,
) -> Result<Json<Value>> {
    submit(state, headers, id, input, true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // The browser suite stubs the Thelxinoe API, so it cannot catch Prowlarr
    // error translation bugs. Nameless authentication errors must retain their
    // meaning, blank field names must not produce an empty instruction, network
    // failures must not be called bad keys, and upstream text must not expose
    // submitted secrets. These cases are exercised before changing the mapper.
    #[test]
    fn translates_indexer_authentication_without_echoing_upstream_text() {
        for error in [
            json!({"propertyName":"", "errorMessage":"Unable to connect to indexer. Incorrect user credentials (wrong API key)"}),
            json!({"propertyName":"", "errorMessage":"Indexer returned result for RSS URL, Credentials appears to be invalid. Response: fixture-secret"}),
            json!({"propertyName":"ApiKey", "errorMessage":"Invalid value: fixture-secret"}),
        ] {
            let message = validation_message(&json!([error]), true);
            assert_eq!(
                message,
                "Invalid API key. Check your indexer API key and try again."
            );
            assert!(!message.contains("fixture-secret"));
        }
        assert_eq!(
            validation_message(
                &json!([{"propertyName":"", "errorMessage":"Indexer returned result for RSS URL, Credentials appears to be invalid. Response: fixture-secret"}]),
                false
            ),
            "The indexer rejected your login credentials. Check them and try again."
        );
    }

    #[test]
    fn empty_or_network_errors_do_not_claim_the_api_key_is_invalid() {
        for errors in [
            Value::Null,
            json!([]),
            json!([{"propertyName":"", "errorMessage":"Unable to connect to indexer. DNS failure at https://example.test/?apikey=fixture-secret"}]),
        ] {
            let message = validation_message(&errors, true);
            assert_eq!(
                message,
                "Prowlarr could not connect to this indexer. Check its address and connection settings, then try again."
            );
        }
        assert_eq!(
            validation_message(
                &json!([{"propertyName":""},{"propertyName":"BaseSettings.QueryLimit"}]),
                true
            ),
            "Check these indexer settings: BaseSettings.QueryLimit"
        );
    }
}
