use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf},
    net::windows::named_pipe::NamedPipeClient,
    sync::{Mutex, mpsc, oneshot},
    task::JoinHandle,
};

type Pending = Arc<Mutex<Option<(u64, oneshot::Sender<Value>)>>>;

// Read replies separately from events: an awaited command must not stall the
// event reader or depend on the timing of playlist property notifications.
pub struct Ipc {
    writer: WriteHalf<NamedPipeClient>,
    pending: Pending,
    serial: u64,
    reader: JoinHandle<()>,
}
impl Ipc {
    pub fn new(client: NamedPipeClient) -> (Self, mpsc::Receiver<Value>) {
        let (reader, writer) = tokio::io::split(client);
        let pending: Pending = Arc::new(Mutex::new(None));
        let replies = pending.clone();
        let (events, receiver) = mpsc::channel(512);
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            loop {
                // A misbehaving custom installation cannot grow a line without limit.
                let mut line = Vec::new();
                loop {
                    let Ok(bytes) = reader.fill_buf().await else {
                        return;
                    };
                    if bytes.is_empty() {
                        return;
                    }
                    let end = bytes.iter().position(|b| *b == b'\n');
                    let count = end.map_or(bytes.len(), |n| n + 1);
                    if line.len() + count > 1024 * 1024 {
                        return;
                    }
                    line.extend_from_slice(&bytes[..count]);
                    reader.consume(count);
                    if end.is_some() {
                        break;
                    }
                }
                let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(id) = message["request_id"].as_u64() {
                    let mut pending = replies.lock().await;
                    if pending.as_ref().is_some_and(|(serial, _)| *serial == id)
                        && let Some((_, reply)) = pending.take()
                    {
                        let _ = reply.send(message);
                    }
                } else if message["event"].is_string() && events.try_send(message).is_err() {
                    // Fail closed if the consumer cannot keep up; never accumulate
                    // an unbounded event backlog or discard a terminal event.
                    return;
                }
            }
        });
        (
            Self {
                writer,
                pending,
                serial: 0,
                reader,
            },
            receiver,
        )
    }
    pub async fn call(&mut self, command: Value) -> Result<Value> {
        self.serial += 1;
        let (reply, receiver) = oneshot::channel();
        *self.pending.lock().await = Some((self.serial, reply));
        let mut bytes = serde_json::to_vec(&json!({"command":command,"request_id":self.serial}))?;
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .await
            .context("MPV control connection closed")?;
        let message = tokio::time::timeout(Duration::from_secs(10), receiver)
            .await
            .context("MPV did not acknowledge its command")?
            .context("MPV control connection closed")?;
        // Do not include MPV responses or URLs in errors: they can contain grants.
        ensure!(
            message["error"] == "success",
            "MPV rejected the playback command"
        );
        Ok(message["data"].clone())
    }
}
impl Drop for Ipc {
    fn drop(&mut self) {
        self.reader.abort();
    }
}
