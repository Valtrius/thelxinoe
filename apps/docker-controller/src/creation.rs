use super::*;

pub(super) const ATTEMPT_LABEL: &str = "app.thelxinoe.creation-attempt";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Intent {
    deployment: String,
    #[serde(default)]
    predecessor: Option<String>,
    pub service: Managed,
}

impl Intent {
    pub(super) fn new(d: &Deployment, mut service: Managed, attempt: &str) -> Self {
        let predecessor = (!service.container.is_empty()).then(|| service.container.clone());
        service.container.clear();
        service.expected = Value::Null;
        service.spec["Labels"][ATTEMPT_LABEL] = json!(attempt);
        Self {
            deployment: d.id.clone(),
            predecessor,
            service,
        }
    }

    pub(super) fn recorded(d: &Deployment, service: &Managed) -> Self {
        Self {
            deployment: d.id.clone(),
            predecessor: None,
            service: service.clone(),
        }
    }

    pub(super) async fn find(&self, d: &Deployment) -> Result<Option<String>> {
        if self.deployment != d.id {
            return Err(conflict("Creation intent belongs to another deployment"));
        }
        let s = &self.service;
        let attempt = s.spec["Labels"][ATTEMPT_LABEL]
            .as_str()
            .ok_or_else(|| conflict("Creation attempt is missing"))?;
        let rows = engine("/containers/json?all=true").await?;
        let mut found = None;
        for row in rows.as_array().ok_or_else(unavailable)? {
            if self.predecessor.as_ref().is_some_and(|id| row["Id"] == *id) {
                continue;
            }
            let exact = row["Labels"][ATTEMPT_LABEL] == attempt
                && row["Labels"]["app.thelxinoe.deployment"] == d.id
                && row["Labels"]["app.thelxinoe.managed-id"] == s.id;
            let named = row["Names"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|name| name.trim_start_matches('/') == s.name);
            if named && !exact {
                return Err(conflict("Creation name belongs to another attempt"));
            }
            if exact {
                if found.is_some() {
                    return Err(conflict("Multiple containers claim this creation attempt"));
                }
                let id = row["Id"].as_str().ok_or_else(unavailable)?.to_owned();
                let raw = engine(&format!("/containers/{id}/json")).await?;
                verify_recorded(d, s, &raw).await?;
                found = Some(id);
            }
        }
        Ok(found)
    }

    // The caller must persist this intent before submitting creation and persist
    // the verified ID before allowing any production start request.
    pub(super) async fn create(&self, d: &Deployment) -> Result<String> {
        if let Some(id) = self.find(d).await? {
            return Ok(id);
        }
        let created = request(
            reqwest::Method::POST,
            &format!("/containers/create?name={}", self.service.name),
            Some(self.service.spec.clone()),
        )
        .await;
        let found = self.find(d).await?;
        match (created, found) {
            (Ok(reply), Some(id)) if reply["Id"] == id => Ok(id),
            (Err(_), Some(id)) => Ok(id),
            (Err(error), None) => Err(error),
            _ => Err(conflict("Docker creation identity could not be verified")),
        }
    }
}
