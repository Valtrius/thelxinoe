//! Two isolated resident Streamlink workers, bounded by the shared extraction slots.
use anyhow::{Context, Result, bail, ensure};
use process_wrap::tokio::*;
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::Mutex,
};

const PYTHON: &str = "/opt/streamlink/bin/python";
const SCRIPT: &str = include_str!("streamlink_worker.py");

#[derive(Default)]
pub(super) struct Pool {
    workers: [Mutex<Option<Worker>>; 2],
}

impl Pool {
    pub async fn warm(&self) {
        if !Path::new(PYTHON).is_file() {
            return;
        }
        for worker in &self.workers {
            let mut worker = worker.lock().await;
            if worker.is_none() {
                *worker = Worker::start().await.ok();
            }
        }
    }

    pub async fn resolve(&self, provider: &str, channel: &str) -> Result<String> {
        let mut slot = if let Some(slot) = self
            .workers
            .iter()
            .find_map(|worker| worker.try_lock().ok())
        {
            slot
        } else {
            self.workers[0].lock().await
        };
        // Cancellation kills the owned worker; its pending response must never
        // be mistaken for the next caller's response.
        let mut worker = match slot.take() {
            Some(worker) => worker,
            None => Worker::start().await?,
        };
        let result = tokio::time::timeout(Duration::from_secs(45), async {
            match worker.resolve(provider, channel).await {
                Ok(value) => Ok(value),
                Err(_) => {
                    // A resident process may have exited between requests. Retry
                    // a broken transport once, within the same request deadline.
                    worker.stop().await;
                    worker = Worker::start().await?;
                    worker.resolve(provider, channel).await
                }
            }
        })
        .await;
        match result {
            Ok(Ok(value)) => {
                *slot = Some(worker);
                value["url"]
                    .as_str()
                    .map(str::to_owned)
                    .context("Public stream extraction failed")
            }
            _ => {
                worker.stop().await;
                bail!("Public stream extraction failed or timed out")
            }
        }
    }
}

struct Worker {
    child: Box<dyn ChildWrapper>,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    _home: tempfile::TempDir,
}

impl Worker {
    async fn start() -> Result<Self> {
        let home = tempfile::tempdir()?;
        let mut command = CommandWrap::with_new(PYTHON, |cmd| {
            cmd.args(["-I", "-u", "-c", SCRIPT])
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
            ] {
                cmd.env(key, home.path());
            }
            cmd.env("LANG", "C.UTF-8");
        });
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(JobObject);
        let mut child = command.spawn().context("Start Streamlink worker")?;
        let input = child.stdin().take().context("Missing worker input")?;
        let output = BufReader::new(child.stdout().take().context("Missing worker output")?);
        let mut worker = Self {
            child,
            input,
            output,
            _home: home,
        };
        let ready =
            tokio::time::timeout(Duration::from_secs(10), response(&mut worker.output)).await??;
        ensure!(ready["ready"] == true, "Worker did not become ready");
        Ok(worker)
    }

    async fn resolve(&mut self, provider: &str, channel: &str) -> Result<Value> {
        let mut request = serde_json::to_vec(&json!({"provider":provider,"channel":channel}))?;
        request.push(b'\n');
        self.input.write_all(&request).await?;
        self.input.flush().await?;
        response(&mut self.output).await
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
            bytes.len() + count <= 32 * 1024,
            "Worker response exceeded its limit"
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
    async fn framing_preserves_next_response_and_rejects_truncated_or_unbounded_output() {
        let mut input = &b"{\"ready\":true}\n{\"url\":\"https://example.test/stream\"}\n"[..];
        assert_eq!(response(&mut input).await.unwrap()["ready"], true);
        assert!(response(&mut input).await.unwrap()["url"].is_string());
        assert!(response(&mut &b"{\"ready\":true}"[..]).await.is_err());
        assert!(response(&mut &vec![b'a'; 32769][..]).await.is_err());
    }
}
