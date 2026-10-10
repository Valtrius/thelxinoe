use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub tool: String,
    pub channel: String,
    pub version: String,
    pub platform: String,
    pub notes_url: String,
    pub source: String,
    pub license: String,
    pub artifacts: Vec<serde_json::Value>,
    pub python_abi: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation {
    pub id: String,
    pub candidate: Candidate,
    pub files: BTreeMap<String, String>,
    pub executables: BTreeMap<String, String>,
    pub python_abi: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Executable {
    pub version: String,
    pub path: PathBuf,
    pub digest: String,
}
impl Executable {
    pub async fn verify(&self) -> Result<()> {
        use tokio::io::AsyncReadExt;
        let mut input = tokio::fs::File::open(&self.path).await?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 128 * 1024];
        loop {
            let size = input.read(&mut buffer).await?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        let digest: String = hash.finalize().iter().map(|v| format!("{v:02x}")).collect();
        ensure!(
            digest == self.digest,
            "Managed tool integrity failed; repair the affected tool"
        );
        Ok(())
    }
    /// `verify`, without rehashing a file that is provably unchanged since it
    /// last matched: any write updates its inode change time, which callers
    /// cannot set. Hashing large tools on slow mounts takes seconds per use.
    pub async fn verify_unchanged(&self) -> Result<()> {
        let key = (self.path.clone(), self.digest.clone());
        let before = fingerprint(&self.path).await?;
        if before.is_some() && VERIFIED.lock().unwrap().get(&key) == before.as_ref() {
            return Ok(());
        }
        self.verify().await?;
        // Only a file left unchanged while hashing proves what was hashed.
        if let Some(before) = before
            && fingerprint(&self.path).await? == Some(before)
        {
            VERIFIED.lock().unwrap().insert(key, before);
        }
        Ok(())
    }
}

type Fingerprint = [i64; 7];
static VERIFIED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<(PathBuf, String), Fingerprint>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(unix)]
async fn fingerprint(path: &Path) -> Result<Option<Fingerprint>> {
    use std::os::unix::fs::MetadataExt;
    let m = tokio::fs::metadata(path).await?;
    Ok(Some([
        m.dev() as i64,
        m.ino() as i64,
        m.len() as i64,
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ]))
}
#[cfg(not(unix))]
async fn fingerprint(_: &Path) -> Result<Option<Fingerprint>> {
    Ok(None)
}

#[derive(Clone, Debug)]
pub struct Package {
    pub generation: Arc<Generation>,
    root: PathBuf,
}
impl Package {
    pub fn executable(&self, name: &str) -> Result<Executable> {
        let relative = self
            .generation
            .executables
            .get(name)
            .context("Package executable is absent")?;
        ensure!(
            Path::new(relative)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_))),
            "Unsafe package path"
        );
        let digest = self
            .generation
            .files
            .get(relative)
            .context("Executable has no verified digest")?;
        Ok(Executable {
            version: self.generation.candidate.version.clone(),
            path: self.root.join(relative),
            digest: digest.clone(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct MediaTools {
    pub ffmpeg: Executable,
    pub ffprobe: Executable,
    pub package: Package,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnlineSnapshot {
    pub yt_dlp: Executable,
    pub module: Executable,
    pub deno: Executable,
    pub ffmpeg: Executable,
    pub ffprobe: Executable,
    pub generations: Vec<String>,
    #[serde(skip)]
    pub leases: Vec<Arc<Generation>>,
}

impl PartialEq for OnlineSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.yt_dlp == other.yt_dlp && self.module == other.module && self.deno == other.deno
    }
}
impl Eq for OnlineSnapshot {}

#[derive(Default)]
struct Inventory {
    selected: BTreeMap<String, String>,
    packages: BTreeMap<String, Arc<Generation>>,
}
#[derive(Clone, Default)]
pub struct Runtime {
    root: PathBuf,
    inventory: Arc<RwLock<Inventory>>,
}
impl Runtime {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            ..Self::default()
        }
    }
    pub fn publish(&self, generations: Vec<Generation>, selected: BTreeMap<String, String>) {
        let mut inventory = self.inventory.write().expect("Tool inventory lock");
        for generation in generations {
            inventory
                .packages
                .entry(generation.id.clone())
                .or_insert_with(|| Arc::new(generation));
        }
        inventory.selected = selected;
    }
    pub fn package(&self, tool: &str) -> Result<Package> {
        let inventory = self.inventory.read().expect("Tool inventory lock");
        let id = inventory.selected.get(tool).with_context(|| {
            format!("{tool} is unavailable; install or repair it in Server tools")
        })?;
        let generation = inventory
            .packages
            .get(id)
            .context("Selected tool package is absent")?
            .clone();
        Ok(Package {
            generation,
            root: self.root.join("packages").join(id),
        })
    }
    pub fn media(&self) -> Result<MediaTools> {
        let package = self.package("ffmpeg")?;
        Ok(MediaTools {
            ffmpeg: package.executable("ffmpeg")?,
            ffprobe: package.executable("ffprobe")?,
            package,
        })
    }
    pub fn online(&self) -> Result<OnlineSnapshot> {
        let inventory = self.inventory.read().expect("Tool inventory lock");
        let mut leases = Vec::new();
        let mut packages = Vec::new();
        for tool in ["yt-dlp", "deno", "ffmpeg"] {
            let id = inventory
                .selected
                .get(tool)
                .with_context(|| format!("{tool} is unavailable"))?;
            let generation = inventory
                .packages
                .get(id)
                .context("Missing tool generation")?
                .clone();
            leases.push(generation.clone());
            packages.push(Package {
                generation,
                root: self.root.join("packages").join(id),
            });
        }
        Ok(OnlineSnapshot {
            yt_dlp: packages[0].executable("yt_dlp")?,
            module: packages[0].executable("module")?,
            deno: packages[1].executable("deno")?,
            ffmpeg: packages[2].executable("ffmpeg")?,
            ffprobe: packages[2].executable("ffprobe")?,
            generations: leases.iter().map(|g| g.id.clone()).collect(),
            leases,
        })
    }
    pub fn restore_online(&self, snapshot: &mut OnlineSnapshot) -> Result<()> {
        let inventory = self.inventory.read().expect("Tool inventory lock");
        snapshot.leases = snapshot
            .generations
            .iter()
            .map(|id| {
                inventory
                    .packages
                    .get(id)
                    .cloned()
                    .context("Queued download tool generation is unavailable")
            })
            .collect::<Result<_>>()?;
        // Paths are server-owned, persisted only with a known immutable generation.
        for executable in [
            &mut snapshot.yt_dlp,
            &mut snapshot.module,
            &mut snapshot.deno,
            &mut snapshot.ffmpeg,
            &mut snapshot.ffprobe,
        ] {
            let mut resolved = None;
            for generation in &snapshot.leases {
                let package = Package {
                    generation: generation.clone(),
                    root: self.root.join("packages").join(&generation.id),
                };
                for name in generation.executables.keys() {
                    let candidate = package.executable(name)?;
                    if candidate.digest == executable.digest {
                        resolved = Some(candidate);
                    }
                }
            }
            *executable =
                resolved.context("Queued executable is not part of its tool generation")?;
        }
        Ok(())
    }
    pub fn referenced(&self) -> Vec<String> {
        self.inventory
            .read()
            .expect("Tool inventory lock")
            .packages
            .iter()
            .filter(|(_, g)| Arc::strong_count(g) > 1)
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn forget(&self, id: &str) -> bool {
        let mut inventory = self.inventory.write().expect("Tool inventory lock");
        if inventory.selected.values().any(|value| value == id)
            || inventory
                .packages
                .get(id)
                .is_some_and(|g| Arc::strong_count(g) > 1)
        {
            return false;
        }
        inventory.packages.remove(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unchanged_verification_still_detects_modified_tools() {
        let path = std::env::temp_dir().join(format!("thelxinoe-tool-{}", std::process::id()));
        std::fs::write(&path, b"original tool").unwrap();
        let digest = Sha256::digest(b"original tool")
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect();
        let tool = Executable {
            version: "1".into(),
            path: path.clone(),
            digest,
        };
        tool.verify_unchanged().await.unwrap();
        tool.verify_unchanged().await.unwrap();
        // Same length and restored modification time: only the content differs.
        // Inode times can be as coarse as a scheduler tick.
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        std::fs::write(&path, b"replaced tool").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        assert!(tool.verify_unchanged().await.is_err());
        std::fs::remove_file(path).unwrap();
    }
}
