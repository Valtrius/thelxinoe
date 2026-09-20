use super::*;

impl ToolManager {
    pub async fn mpv_schema(&self, settings: &AppSettings) -> AppResult<MpvSchema> {
        let (settings, _lease) = self
            .resolve_launch(settings, &[crate::tools::models::ToolId::Mpv], false)
            .await?;
        let check = self.probe("mpv", settings.mpv_path.as_deref()).await;
        let path = check.path.filter(|_| check.detected).ok_or_else(|| {
            AppError::validation("Select or install MPV to load its available options.")
        })?;
        let mut probe = Probe::start(&path, &["--no-config".into()]).await?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let names = probe.request(json!(["get_property", "options"])).await?;
            let names = names.as_array().ok_or_else(|| {
                AppError::validation("This MPV version did not expose an option list.")
            })?;
            let mut options = Vec::new();
            for name in names.iter().filter_map(Value::as_str).take(3000) {
                if let Ok(mut info) = probe
                    .request(json!(["get_property", format!("option-info/{name}")]))
                    .await
                    && info.is_object()
                {
                    info["name"] = json!(name);
                    options.push(info);
                }
            }
            Ok::<_, AppError>(MpvSchema {
                executable: path,
                version: check.version.unwrap_or_default(),
                options,
            })
        })
        .await
        .map_err(|_| {
            AppError::validation(
                "MPV option discovery timed out. The raw editor remains available.",
            )
        })?;
        let _ = probe.finish().await;
        result
    }

    pub async fn test_mpv_configuration(
        &self,
        settings: &AppSettings,
        clean: bool,
    ) -> AppResult<String> {
        let (settings, runtime) = self
            .resolve_launch(settings, &[crate::tools::models::ToolId::Mpv], !clean)
            .await?;
        let check = self.probe("mpv", settings.mpv_path.as_deref()).await;
        let path = check
            .path
            .filter(|_| check.detected)
            .ok_or_else(|| AppError::validation("Select a working MPV executable first."))?;
        let args = if clean {
            vec!["--no-config".into()]
        } else {
            runtime.mpv_args.clone()
        };
        let mut probe = Probe::start_mode(&path, &args, !clean).await?;
        probe
            .request(json!(["get_property", "mpv-version"]))
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let log = probe.finish().await?;
        if log.lines().any(|line| {
            let line = line.to_ascii_lowercase();
            line.contains("error")
                || line.contains("failed")
                || line.contains("stack traceback")
                || line.contains("invalid value")
        }) {
            return Err(AppError::validation(format!(
                "MPV configuration check: {}",
                log.chars().take(3000).collect::<String>()
            )));
        }
        Ok(if clean {
            "MPV started successfully with a clean configuration."
        } else {
            "MPV started successfully with the selected configuration and scripts."
        }
        .into())
    }
}
