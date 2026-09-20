use super::*;

pub(super) fn fake_install(manager: &ToolManager, id: ToolId, version: &str) {
    let mut package = providers::test_package(id);
    package.id = version.into();
    let directory = format!("packages/{version}");
    fs::create_dir_all(manager.root.join(&directory)).unwrap();
    manager
        .edit(|s| {
            s.installed.push(InstalledPackage {
                package,
                installed_at: crate::utils::utc_now(),
                directory,
                version_output: "test".into(),
            });
            Ok(())
        })
        .unwrap();
}

pub(super) fn retention_install(manager: &ToolManager, tool: ToolId, number: u8) -> String {
    let id = format!("{}-{number:02}", tool.key());
    fake_install(manager, tool, &id);
    manager
        .edit(|state| {
            let installed = state
                .installed
                .iter_mut()
                .find(|i| i.package.id == id)
                .unwrap();
            installed.installed_at = format!("2026-01-01T00:00:{number:02}Z");
            Ok(())
        })
        .unwrap();
    fs::write(
        manager.root.join("packages").join(&id).join("tool.exe"),
        b"test package",
    )
    .unwrap();
    id
}
