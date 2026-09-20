//! Two resident yt-dlp instances share the existing public extraction limit.
use super::tools::{self, Bundle, Executable};
use crate::AppState;
use anyhow::{Context, Result, ensure};
use process_wrap::tokio::*;
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{Mutex, RwLock},
};

const PYTHON: &str = "/opt/streamlink/bin/python";
const SCRIPT: &str = include_str!("youtube_worker.py");
struct Snapshot {
    bundle: Bundle,
    module: Executable,
}

#[derive(Default)]
pub(super) struct Pool {
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    workers: [Mutex<Option<Worker>>; 2],
}
impl Pool {
    pub async fn run(&self, state: &AppState) -> Result<()> {
        if !Path::new(PYTHON).is_file() {
            return Ok(());
        }
        loop {
            if let Ok(Some(bundle)) = tools::selection(state).await
                && self.warm(state, bundle).await.is_err()
            {
                // CLI playback remains available; never log extractor output.
                tracing::warn!("Resident YouTube extraction is unavailable; using CLI playback");
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }
    async fn warm(&self, state: &AppState, bundle: Bundle) -> Result<()> {
        let current = self.snapshot.read().await.clone();
        let snapshot = if let Some(snapshot) = current.filter(|s| s.bundle == bundle) {
            snapshot
        } else {
            let module = tools::python_module(state, &bundle).await?;
            tools::verify(&bundle.deno).await?;
            Arc::new(Snapshot { bundle, module })
        };
        for slot in &self.workers {
            let Ok(mut slot) = slot.try_lock() else {
                continue;
            };
            if let Some(worker) = slot.as_mut()
                && worker.snapshot.bundle == snapshot.bundle
                && worker.child.try_wait()?.is_none()
            {
                continue;
            }
            if let Some(mut worker) = slot.take() {
                worker.stop().await;
            }
            *slot = Some(Worker::start(Path::new(PYTHON), snapshot.clone()).await?);
        }
        *self.snapshot.write().await = Some(snapshot);
        Ok(())
    }
    /// None selects the existing CLI while a matching module is being prepared.
    /// A worker's provider errors are responses, not reasons to retry via CLI.
    pub async fn resolve(&self, bundle: &Bundle, id: &str) -> Result<Option<Value>> {
        let Some(snapshot) = self
            .snapshot
            .read()
            .await
            .clone()
            .filter(|s| s.bundle == *bundle)
        else {
            return Ok(None);
        };
        // The Deno executable is still invoked by the resident extractor; retain
        // the same tamper check before every extraction, including a reused worker.
        tools::verify(&snapshot.module).await?;
        tools::verify(&bundle.deno).await?;
        let mut slot = if let Some(slot) = self.workers.iter().find_map(|w| w.try_lock().ok()) {
            slot
        } else {
            self.workers[0].lock().await
        };
        let mut worker = match slot.take() {
            Some(worker) if worker.snapshot.bundle == *bundle => worker,
            old => {
                if let Some(mut old) = old {
                    old.stop().await;
                }
                Worker::start(Path::new(PYTHON), snapshot.clone()).await?
            }
        };
        // The request owns the process until its response has been consumed.
        // Cancellation drops and kills it, including any Deno descendants.
        let response = tokio::time::timeout(Duration::from_secs(120), async {
            match worker.resolve(id).await {
                Ok(response) => Ok(response),
                Err(_) => {
                    worker.stop().await;
                    worker = Worker::start(Path::new(PYTHON), snapshot.clone()).await?;
                    worker.resolve(id).await
                }
            }
        })
        .await;
        match response {
            Ok(Ok(value)) => {
                // Bound caches retained inside third-party extractor instances.
                if worker.requests < 100 {
                    *slot = Some(worker);
                } else {
                    worker.stop().await;
                }
                Ok(Some(value))
            }
            _ => {
                worker.stop().await;
                anyhow::bail!("Public extraction failed or timed out");
            }
        }
    }
}
struct Worker {
    snapshot: Arc<Snapshot>,
    child: Box<dyn ChildWrapper>,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    requests: usize,
    _home: tempfile::TempDir,
}
impl Worker {
    async fn start(python: &Path, snapshot: Arc<Snapshot>) -> Result<Self> {
        let home = tempfile::tempdir()?;
        let mut command = CommandWrap::with_new(python, |cmd| {
            cmd.args(["-I", "-u", "-c", SCRIPT])
                .arg(&snapshot.module.path)
                .arg(&snapshot.bundle.deno.path)
                .arg(&snapshot.bundle.yt_dlp.version)
                .env_clear()
                .current_dir(home.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            for key in [
                "HOME",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "TMPDIR",
                "TMP",
                "TEMP",
                "DENO_DIR",
            ] {
                cmd.env(key, home.path());
            }
            cmd.env("LANG", "C.UTF-8");
            #[cfg(windows)]
            for key in ["SystemRoot", "WINDIR"] {
                if let Some(value) = std::env::var_os(key) {
                    cmd.env(key, value);
                }
            }
        });
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(JobObject).wrap(CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        let mut child = command.spawn().context("Start YouTube worker")?;
        let input = child.stdin().take().context("Missing worker input")?;
        let output = BufReader::new(child.stdout().take().context("Missing worker output")?);
        let mut worker = Self {
            snapshot,
            child,
            input,
            output,
            requests: 0,
            _home: home,
        };
        let ready =
            tokio::time::timeout(Duration::from_secs(10), response(&mut worker.output)).await??;
        ensure!(
            ready["ready"] == true && ready["version"] == worker.snapshot.bundle.yt_dlp.version,
            "Unexpected extractor module version"
        );
        Ok(worker)
    }
    async fn resolve(&mut self, id: &str) -> Result<Value> {
        self.input
            .write_all(format!("{}\n", json!({"id":id})).as_bytes())
            .await?;
        self.input.flush().await?;
        let value = response(&mut self.output).await?;
        if let Some(metadata) = value.get("metadata") {
            ensure!(metadata["id"] == id, "Extractor returned a different video");
        } else {
            ensure!(
                matches!(
                    value["error"].as_str(),
                    Some("extractor_authentication_required" | "unavailable" | "extraction_failed")
                ),
                "Invalid extractor response"
            );
        }
        self.requests += 1;
        Ok(value)
    }
    async fn stop(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}
async fn response(input: &mut (impl AsyncBufRead + Unpin)) -> Result<Value> {
    let mut bytes = Vec::new();
    loop {
        let buffer = input.fill_buf().await?;
        ensure!(!buffer.is_empty(), "Worker closed its output");
        let count = buffer
            .iter()
            .position(|v| *v == b'\n')
            .map_or(buffer.len(), |n| n + 1);
        ensure!(
            bytes.len() + count <= 8 * 1024 * 1024,
            "Extractor response exceeded its limit"
        );
        bytes.extend_from_slice(&buffer[..count]);
        input.consume(count);
        if bytes.last() == Some(&b'\n') {
            return Ok(serde_json::from_slice(&bytes)?);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_preserves_the_next_response() {
        let mut input =
            BufReader::with_capacity(3, &b"{\"ready\":true}\n{\"error\":\"unavailable\"}\n"[..]);
        assert_eq!(response(&mut input).await.unwrap(), json!({"ready":true}));
        assert_eq!(
            response(&mut input).await.unwrap(),
            json!({"error":"unavailable"})
        );
        assert!(response(&mut input).await.is_err());
    }

    #[tokio::test]
    async fn framing_rejects_truncated_invalid_and_excessive_output() {
        for bytes in [
            b"{\"ready\":true}".to_vec(),
            b"not json\n".to_vec(),
            vec![b' '; 8 * 1024 * 1024 + 1],
        ] {
            assert!(
                response(&mut BufReader::new(bytes.as_slice()))
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn a_changed_bundle_uses_cli_until_its_own_module_is_ready() {
        let executable = Executable {
            version: "old".into(),
            path: "missing".into(),
            digest: String::new(),
        };
        let mut bundle = Bundle {
            yt_dlp: executable.clone(),
            deno: executable.clone(),
        };
        let pool = Pool::default();
        assert!(
            pool.resolve(&bundle, "9pkqztOC1WM")
                .await
                .unwrap()
                .is_none()
        );
        *pool.snapshot.write().await = Some(Arc::new(Snapshot {
            bundle: bundle.clone(),
            module: executable,
        }));
        // Matching snapshots still fail closed when their verified files vanish.
        assert!(pool.resolve(&bundle, "9pkqztOC1WM").await.is_err());
        bundle.yt_dlp.version = "new".into();
        assert!(
            pool.resolve(&bundle, "9pkqztOC1WM")
                .await
                .unwrap()
                .is_none()
        );
        bundle.yt_dlp.version = "old".into();
        bundle.deno.path = "replacement".into();
        assert!(
            pool.resolve(&bundle, "9pkqztOC1WM")
                .await
                .unwrap()
                .is_none()
        );
    }
}
