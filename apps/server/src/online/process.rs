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
    home: tempfile::TempDir,
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
    output_limit: usize,
) -> anyhow::Result<Output> {
    run_progress(home, executable, args, timeout, output_limit, None).await
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
    home: tempfile::TempDir,
    executable: &Path,
    args: &[OsString],
    timeout: Duration,
    output_limit: usize,
    progress: Option<tokio::sync::mpsc::Sender<String>>,
) -> anyhow::Result<Output> {
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
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows::Win32::{
            Foundation::{HANDLE, WAIT_OBJECT_0},
            System::Threading::{
                OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
                WaitForSingleObject,
            },
        };

        // A native fixture exercises the runner without PowerShell/.NET startup
        // and module initialization inside the deliberately empty environment.
        let fixture = tempfile::tempdir().unwrap();
        let executable = fixture.path().join("online-process.exe");
        let build =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg("--edition=2024")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/helpers/online-process.rs"
                ))
                .arg("-o")
                .arg(&executable)
                .output()
                .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let output = run(
            tempfile::tempdir().unwrap(),
            &executable,
            &[],
            Duration::from_secs(60),
            1024,
        )
        .await
        .unwrap();
        assert!(
            output.success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "isolated");

        let marker = tempfile::tempdir().unwrap();
        let ready = marker.path().join("child.pid");
        let directory = marker.path().as_os_str().to_owned();
        let task = tokio::spawn(async move {
            run(
                tempfile::tempdir().unwrap(),
                &executable,
                &["tree".into(), directory],
                Duration::from_secs(60),
                1024,
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(60), async {
            while !ready.exists() {
                assert!(
                    !task.is_finished(),
                    "Native fixture exited before its child was ready"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Native child did not start");
        let pid = std::fs::read_to_string(ready).unwrap().parse().unwrap();
        // Capture the live process handle before allowing the parent to overflow.
        // Waiting on that handle proves this descendant exited, without PID reuse.
        let handle =
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }.unwrap();
        let process = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        std::fs::write(marker.path().join("release"), b"").unwrap();
        let result = task.await.unwrap();
        let exited = unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 5000) };
        if exited != WAIT_OBJECT_0 {
            unsafe { TerminateProcess(HANDLE(process.as_raw_handle()), 1) }.unwrap();
        }
        assert_eq!(exited, WAIT_OBJECT_0, "Descendant escaped the Windows job");
        let failure = result.err().expect("Output overflow must stop the tool");
        assert!(failure.to_string().contains("output exceeded"), "{failure}");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn isolated_environment_and_timeout() {
        let output = run(
            tempfile::tempdir().unwrap(),
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
                tempfile::tempdir().unwrap(),
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
