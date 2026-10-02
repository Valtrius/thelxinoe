use super::*;

pub(in crate::managers) async fn profiles(state: &AppState, service: &str) -> Result<Vec<Profile>> {
    Ok(storage::profiles(&state.db, service.into()).await?)
}
pub(super) async fn fetch_catalog(state: &AppState, key: &str, kind: &str) -> Result<Value> {
    controller_request(
        state,
        &format!("/{key}/recyclarr/catalog"),
        Some(json!({"kind":kind})),
    )
    .await
}
pub(super) async fn catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
) -> Result<Json<Value>> {
    security::require(&state, &headers, Capability::ManageServer).await?;
    Ok(Json(
        fetch_catalog(&state, &provision(&state).await?, &kind).await?,
    ))
}
pub(super) async fn select(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(input): Json<Selection>,
) -> Result<Json<Value>> {
    let actor = security::require(&state, &headers, Capability::ManageServer).await?;
    let s = service(&state, &key).await?;
    if current_configuration(&state, &provision(&state).await?).await?["mode"] == "customized" {
        return Err(ApiError::conflict(
            "Edit guide profiles in Recyclarr YAML while using a customized configuration",
        ));
    }
    let catalog = fetch_catalog(&state, &provision(&state).await?, &s.kind).await?;
    let selected = catalog["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["trash_id"] == input.trash_id)
        .ok_or_else(|| ApiError::bad("Choose a current guide profile"))?;
    if !input.overrides.as_object().is_some_and(|m| {
        m.iter().all(|(k, v)| match k.as_str() {
            "min_format_score" | "upgrade_until_score" => {
                v.as_i64().is_some_and(|v| (-100000..=100000).contains(&v))
            }
            "upgrade_allowed" => v.is_boolean(),
            _ => false,
        })
    }) {
        return Err(ApiError::bad("Unsupported guide override"));
    }
    if !input
        .groups
        .as_object()
        .is_some_and(|m| m.keys().all(|k| matches!(k.as_str(), "add" | "skip")))
    {
        return Err(ApiError::bad("Invalid custom-format groups"));
    }
    for field in ["add", "skip"] {
        let rows = input.groups[field]
            .as_array()
            .ok_or_else(|| ApiError::bad("Choose compatible custom-format groups"))?;
        for value in rows {
            let trash = if field == "add" {
                value["trash_id"].as_str()
            } else {
                value.as_str()
            };
            if trash.is_none()
                || !selected["groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|g| g["trash_id"].as_str() == trash)
                || (field == "add" && value.as_object().is_none_or(|m| m.len() != 1))
            {
                return Err(ApiError::bad(
                    "Group is incompatible with this guide profile",
                ));
            }
        }
    }
    let run = storage::select(&state.db, key, input, actor.user.id).await?;
    Ok(Json(json!({"id":run,"state":"queued"})))
}
