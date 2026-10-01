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
    run_progress(executable, args, timeout, output_limit, None).await
}

async fn collect_progress(
    mut pipe: impl AsyncRead + Unpin,
    limit: usize,
    progress: Option<tokio::sync::mpsc::Sender<String>>,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut line = Vec::new();
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
        if let Some(progress) = &progress {
            for byte in &buffer[..count] {
                if *byte == b'\n' {
                    if line.starts_with(b"THELXINOE_PROGRESS:")
                        && let Ok(value) = String::from_utf8(line.clone())
                    {
                        let _ = progress.try_send(value);
                    }
                    line.clear();
                } else if line.len() < 2048 {
                    line.push(*byte);
                }
            }
        }
    }
}

pub(super) async fn run_progress(
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
    output_limit: usize,
    progress: Option<tokio::sync::mpsc::Sender<String>>,
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
            collect_progress(stdout, output_limit, progress),
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
    async fn reports_progress_before_process_output_completes() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(256);
        let (send, mut receive) = tokio::sync::mpsc::channel(2);
        let task = tokio::spawn(collect_progress(reader, 1024, Some(send)));
        writer
            .write_all(b"ordinary output\nTHELXINOE_PROG")
            .await
            .unwrap();
        writer
            .write_all(b"RESS:25\t100\tNA\t3\th264\tnone\n")
            .await
            .unwrap();
        let progress = tokio::time::timeout(Duration::from_secs(10), receive.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(progress, "THELXINOE_PROGRESS:25\t100\tNA\t3\th264\tnone");
        assert!(
            !task.is_finished(),
            "Progress must be delivered while the tool is still running"
        );
        drop(writer);
        assert!(task.await.unwrap().is_ok());
    }
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
        let output = run(&shell, &["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "if ($env:HOME -ne $env:APPDATA -or $env:HOME -ne $env:TEMP -or $env:PATH) { exit 9 }; Write-Output 'isolated'".into()], Duration::from_secs(60), 1024).await.unwrap();
        assert!(output.success);
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "isolated");
        let marker = tempfile::tempdir().unwrap();
        let ready = marker.path().join("child.pid");
        let child_script = marker.path().join("child.ps1");
        std::fs::write(
            &child_script,
            format!(
                "Set-Content -LiteralPath '{}' -Encoding ascii -Value $PID; Start-Sleep 120",
                ready.display()
            ),
        )
        .unwrap();
        // Output overflow triggers the same cleanup path as timeout, after the
        // descendant has acknowledged startup. No fixed startup race or delayed marker.
        let script = format!(
            "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList '-NoProfile -NonInteractive -File \"{}\"'; while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 50 }}; Write-Output ('x' * 2048); Start-Sleep 120",
            shell.display(),
            child_script.display(),
            ready.display()
        );
        let failure = run(
            &shell,
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.into(),
            ],
            Duration::from_secs(60),
            1024,
        )
        .await
        .err()
        .expect("Output overflow must stop the tool");
        assert!(failure.to_string().contains("output exceeded"), "{failure}");
        let pid: u32 = std::fs::read_to_string(ready)
            .unwrap()
            .trim()
            .trim_start_matches('\u{feff}')
            .parse()
            .unwrap();
        let cleanup = format!(
            "$child = Get-Process -Id {pid} -ErrorAction SilentlyContinue; if ($child -and -not $child.WaitForExit(30000)) {{ Stop-Process -InputObject $child -Force; exit 9 }}; Write-Output 'killed'"
        );
        let result = run(
            &shell,
            &[
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                cleanup.into(),
            ],
            Duration::from_secs(60),
            1024,
        )
        .await
        .unwrap();
        assert!(result.success, "Descendant escaped the Windows job");
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
