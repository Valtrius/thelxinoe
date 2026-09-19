//! Durable controller-owned state; no server database is needed to inspect ownership.
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
pub fn root() -> PathBuf {
    PathBuf::from(
        std::env::var("THELXINOE_DEPLOYMENT").unwrap_or("/var/lib/thelxinoe/deployment".into()),
    )
}
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write(path, &bytes)
}
pub fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".write-{}", thelxinoe_core::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn read<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
/// One atomic pointer commits both descriptor and Compose image pins.
pub fn commit_generation(
    root: &Path,
    generation: u64,
    descriptor: &serde_json::Value,
    compose: &serde_json::Value,
) -> anyhow::Result<()> {
    let name = format!("generations/{generation}");
    let directory = root.join(&name);
    anyhow::ensure!(!directory.exists(), "Generation already exists");
    std::fs::create_dir_all(&directory)?;
    write_json(&directory.join("desired-state.json"), descriptor)?;
    write_json(&directory.join("compose.override.yaml"), compose)?;
    for (name, target) in [
        ("desired-state.json", "current/desired-state.json"),
        ("compose.override.yaml", "current/compose.override.yaml"),
    ] {
        let path = root.join(name);
        if std::fs::symlink_metadata(&path).is_ok() {
            anyhow::ensure!(
                std::fs::read_link(&path)? == Path::new(target),
                "Unexpected deployment pointer"
            );
        } else {
            symlink(target, path)?;
        }
    }
    let pointer = root.join(format!(".current-{}", thelxinoe_core::id()));
    symlink(&name, &pointer)?;
    std::fs::rename(pointer, root.join("current"))?;
    File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn descriptor_and_compose_advance_together() {
        let root = std::env::temp_dir().join(thelxinoe_core::id());
        std::fs::create_dir_all(&root).unwrap();
        commit_generation(
            &root,
            1,
            &json!({"generation":1}),
            &json!({"services":{"server":{"image":"sha256:old"}}}),
        )
        .unwrap();
        commit_generation(
            &root,
            2,
            &json!({"generation":2}),
            &json!({"services":{"server":{"image":"sha256:new"}}}),
        )
        .unwrap();
        assert_eq!(
            read::<serde_json::Value>(&root.join("desired-state.json")).unwrap()["generation"],
            2
        );
        assert_eq!(
            read::<serde_json::Value>(&root.join("compose.override.yaml")).unwrap()["services"]["server"]
                ["image"],
            "sha256:new"
        );
        assert!(commit_generation(&root, 2, &json!({}), &json!({})).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
