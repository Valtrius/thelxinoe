use super::*;

pub(super) struct Probe {
    pub(super) child: Child,
    pub(super) _job: ProcessJob,
    pub(super) pipe: BufReader<NamedPipeClient>,
    pub(super) next: u64,
    pub(super) log: tokio::task::JoinHandle<String>,
}

impl Probe {
    pub(super) async fn start(executable: &str, extra: &[String]) -> AppResult<Self> {
        Self::start_mode(executable, extra, false).await
    }
    pub(super) async fn start_mode(
        executable: &str,
        extra: &[String],
        scripts: bool,
    ) -> AppResult<Self> {
        let name = format!(r"\\.\pipe\thelxinoe-options-{}", uuid::Uuid::new_v4());
        let mut command = Command::new(executable);
        command.args(extra);
        if !scripts {
            command.args(["--load-scripts=no", "--scripts-clr"]);
        }
        command
            .args([
                "--idle=yes",
                "--terminal=yes",
                "--input-terminal=no",
                "--msg-color=no",
                "--msg-level=all=warn",
                "--force-window=no",
                "--vo=null",
                "--ao=null",
                "--ytdl=no",
            ])
            .arg(format!("--input-ipc-server={name}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (mut child, job) = ProcessJob::spawn(&mut command)?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::internal("Missing MPV probe stderr"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::internal("Missing MPV probe stdout"))?;
        let log = tokio::spawn(async move {
            let (mut output, mut errors) = (String::new(), String::new());
            let (mut stdout, mut stderr) = (stdout.take(MAX_TEXT), stderr.take(MAX_TEXT));
            let _ = tokio::join!(
                stdout.read_to_string(&mut output),
                stderr.read_to_string(&mut errors)
            );
            output.push_str(&errors);
            output
        });
        let connection = tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if let Ok(pipe) = ClientOptions::new().open(&name) {
                    break Ok(pipe);
                }
                if child.try_wait()?.is_some() {
                    break Err(AppError::validation(
                        "MPV exited while loading its configuration.",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await;
        let pipe = match connection {
            Ok(Ok(pipe)) => pipe,
            _ => {
                job.terminate();
                let _ = child.wait().await;
                let message = log.await.unwrap_or_default();
                return Err(AppError::validation(format!(
                    "MPV configuration check failed. {}",
                    message.chars().take(2000).collect::<String>()
                )));
            }
        };
        Ok(Self {
            child,
            _job: job,
            pipe: BufReader::new(pipe),
            next: 0,
            log,
        })
    }
    pub(super) async fn request(&mut self, command: Value) -> AppResult<Value> {
        self.next += 1;
        let id = self.next;
        let mut bytes = serde_json::to_vec(&json!({"command":command,"request_id":id}))
            .map_err(|e| AppError::internal(e.to_string()))?;
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(3), async {
            self.pipe.get_mut().write_all(&bytes).await?;
            loop {
                let mut line = Vec::new();
                let n = (&mut self.pipe)
                    .take(MAX_TEXT + 1)
                    .read_until(b'\n', &mut line)
                    .await?;
                if n == 0 || n > MAX_TEXT as usize {
                    return Err(AppError::validation(
                        "MPV returned an invalid IPC response.",
                    ));
                }
                let response: Value = serde_json::from_slice(&line)
                    .map_err(|e| AppError::validation(e.to_string()))?;
                if response["request_id"] == id {
                    if response["error"] != "success" {
                        return Err(AppError::validation(format!("MPV: {}", response["error"])));
                    }
                    return Ok(response["data"].clone());
                }
            }
        })
        .await
        .map_err(|_| AppError::validation("MPV did not answer its configuration query."))?
    }
    pub(super) async fn finish(mut self) -> AppResult<String> {
        let _ = self.request(json!(["quit"])).await;
        if tokio::time::timeout(Duration::from_secs(1), self.child.wait())
            .await
            .is_err()
        {
            self._job.terminate();
            let _ = self.child.wait().await;
        }
        Ok(self.log.await.unwrap_or_default())
    }
}
