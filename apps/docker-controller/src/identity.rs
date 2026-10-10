//! Numeric file identities shared by the server, managed services and their workers.
use crate::docker::{Result, engine};
use serde_json::Value;

#[derive(Clone, Copy)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
}

fn numeric(value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0 && *id < i32::MAX as u32)
        .ok_or((
            axum::http::StatusCode::CONFLICT,
            "Container file access requires non-root numeric UID and GID values".into(),
        ))
}

impl Identity {
    pub fn from_user(config: &Value) -> Result<Self> {
        let (uid, gid) = config["User"]
            .as_str()
            .and_then(|user| user.split_once(':'))
            .ok_or::<crate::docker::Failure>((
                axum::http::StatusCode::CONFLICT,
                "Configure the server user as a numeric UID:GID pair".into(),
            ))?;
        Ok(Self {
            uid: numeric(uid)?,
            gid: numeric(gid)?,
        })
    }

    pub fn service(kind: &str, spec: &Value) -> Result<Self> {
        if matches!(kind, "seerr" | "recyclarr") {
            return Self::from_user(spec);
        }
        let variable = |key: &str| {
            spec["Env"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter_map(|entry| entry.split_once('='))
                .find_map(|(name, value)| (name == key).then_some(value))
                // Curated LinuxServer entrypoints default each omitted ID to
                // 911. Adopted services may rely on either or both defaults.
                .unwrap_or("911")
        };
        Ok(Self {
            uid: numeric(variable("PUID"))?,
            gid: numeric(variable("PGID"))?,
        })
    }

    pub fn user(self) -> String {
        format!("{}:{}", self.uid, self.gid)
    }
}

pub async fn prepare_runtime(directory: &std::path::Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let container = std::env::var("HOSTNAME")?;
    let raw = engine(&format!("/containers/{container}/json"))
        .await
        .map_err(|(_, message)| anyhow::anyhow!(message))?;
    let gid = raw["Config"]["User"]
        .as_str()
        .and_then(|user| user.split_once(':'))
        .ok_or_else(|| anyhow::anyhow!("Configure the controller user as 0:GID"))?
        .1;
    let gid = numeric(gid).map_err(|(_, message)| anyhow::anyhow!(message))?;
    // Named volumes initially inherit the image's default group. Only change
    // this private directory; the setgid bit gives new sockets the chosen group.
    if std::fs::metadata(directory)?.gid() != gid {
        std::os::unix::fs::chown(directory, None, Some(gid))?;
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o2770))?;
    Ok(())
}
