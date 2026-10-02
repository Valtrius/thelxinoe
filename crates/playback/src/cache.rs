use std::{io, path::Path};

#[cfg(test)]
type StatGate = (
    std::path::PathBuf,
    tokio::sync::oneshot::Sender<std::path::PathBuf>,
    tokio::sync::oneshot::Receiver<()>,
);

#[derive(Default)]
pub(crate) struct Scanner {
    #[cfg(test)]
    pub(crate) before_stat: tokio::sync::Mutex<Option<StatGate>>,
}

impl Scanner {
    // FFmpeg rotates segments and renames temporary files during enumeration.
    // A removed run/entry contributes no bytes; all other I/O errors remain visible.
    pub(crate) async fn usage(&self, directory: &Path) -> io::Result<u64> {
        let mut entries = match tokio::fs::read_dir(directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error),
        };
        let mut total = 0u64;
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error),
            };
            #[cfg(test)]
            {
                let gate = {
                    let mut gate = self.before_stat.lock().await;
                    if gate
                        .as_ref()
                        .is_some_and(|(target, _, _)| target == directory)
                    {
                        gate.take()
                    } else {
                        None
                    }
                };
                if let Some((_, entered, resume)) = gate {
                    let _ = entered.send(entry.path());
                    let _ = resume.await;
                }
            }
            match entry.metadata().await {
                Ok(meta) => total = total.saturating_add(meta.len()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(total)
    }
}
