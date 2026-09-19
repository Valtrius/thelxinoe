//! Bounded child execution with a fresh configuration environment and tree cleanup.
use anyhow::{Context, ensure};
use process_wrap::tokio::*;
use std::{ffi::OsString, path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

pub(super) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

async fn collect(mut pipe: impl AsyncRead + Unpin, limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            return Ok(bytes);
        }
        ensure!(
            bytes.len() + count <= limit,
            "Online tool output exceeded its limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
    }
}

pub(super) async fn run(
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
    output_limit: usize,
) -> anyhow::Result<Output> {
    let home = tempfile::tempdir()?;
    let mut command = CommandWrap::with_new(executable, |cmd| {
        cmd.args(args)
            .env_clear()
            .current_dir(home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "TMP",
            "TEMP",
            "TMPDIR",
            "DENO_DIR",
        ] {
            cmd.env(key, home.path());
        }
        #[cfg(windows)]
        for key in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        cmd.env("LANG", "C.UTF-8");
    });
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    {
        command.wrap(CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(JobObject);
    }
    let mut child = command
        .spawn()
        .context("Could not start managed online tool")?;
    let stdout = child.stdout().take().context("Missing tool stdout")?;
    let stderr = child.stderr().take().context("Missing tool stderr")?;
    let result = tokio::time::timeout(timeout, async {
        tokio::try_join!(
            async { Ok::<_, anyhow::Error>(child.wait().await?) },
            collect(stdout, output_limit),
            collect(stderr, 256 * 1024),
        )
    })
    .await;
    match result {
        Ok(Ok((status, stdout, stderr))) => Ok(Output {
            success: status.success(),
            stdout,
            stderr,
        }),
        failure => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            match failure {
                Ok(Err(error)) => Err(error),
                _ => anyhow::bail!("Online tool timed out"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn bounded_output_rejects_overflow() {
        assert_eq!(collect(&b"abc"[..], 3).await.unwrap(), b"abc");
        assert!(collect(&b"abcd"[..], 3).await.is_err());
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_environment_isolated_and_tree_killed() {
        let shell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let output = run(&shell, &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "if ($env:HOME -ne $env:APPDATA -or $env:HOME -ne $env:TEMP -or $env:PATH) { exit 9 }; Write-Output 'isolated'".into()], Duration::from_secs(10), 1024).await.unwrap();
        assert!(output.success);
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "isolated");
        let marker = tempfile::tempdir().unwrap();
        let path = marker.path().join("escaped.txt");
        let baseline = format!(
            "Start-Process -Wait -WindowStyle Hidden -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -Command Set-Content -LiteralPath ''{}'' -Value baseline'",
            shell.display(),
            path.display()
        );
        let baseline = run(
            &shell,
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                baseline.into(),
            ],
            Duration::from_secs(10),
            1024,
        )
        .await
        .unwrap();
        assert!(baseline.success);
        assert!(
            path.exists(),
            "The descendant fixture must execute before testing termination"
        );
        std::fs::remove_file(&path).unwrap();
        let script = format!(
            "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep 3; Set-Content -LiteralPath ''{}'' -Value escaped'; Start-Sleep 30",
            shell.display(),
            path.display()
        );
        assert!(
            run(
                &shell,
                &[
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    script.into()
                ],
                Duration::from_secs(1),
                1024
            )
            .await
            .is_err()
        );
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!path.exists(), "Descendant escaped the Windows job");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_environment_and_timeout() {
        let output = run(
            Path::new("/bin/sh"),
            &[
                "-c".into(),
                "printf '%s' \"${PATH-unset}:${GOOGLE_ACCESS_TOKEN-unset}\"; test -d \"$HOME\""
                    .into(),
            ],
            Duration::from_secs(2),
            1024,
        )
        .await
        .unwrap();
        assert!(output.success);
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .ends_with(":unset")
        );
        assert!(
            run(
                Path::new("/bin/sh"),
                &["-c".into(), "sleep 30 & wait".into()],
                Duration::from_millis(100),
                1024
            )
            .await
            .is_err()
        );
    }
}
