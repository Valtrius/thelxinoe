use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use thelxinoe_tools::{Candidate, Generation, Runtime};

// Existing media integration tests explicitly inject their runner's FFmpeg pair.
// Production has no PATH fallback.
pub fn install(runtime: &Runtime, root: &Path) {
    let directory = root.join("packages/test-ffmpeg");
    std::fs::create_dir_all(&directory).unwrap();
    let mut files = BTreeMap::new();
    let mut executables = BTreeMap::new();
    for name in ["ffmpeg", "ffprobe"] {
        let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join(&filename))
            .find(|p| p.is_file())
            .expect("FFmpeg test prerequisite");
        let destination = directory.join(&filename);
        if !destination.exists() {
            std::fs::copy(source, &destination).unwrap();
        }
        let hash: String = Sha256::digest(std::fs::read(destination).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        files.insert(filename.clone(), hash);
        executables.insert(name.into(), filename);
    }
    runtime.publish(
        vec![Generation {
            id: "test-ffmpeg".into(),
            files,
            executables,
            python_abi: String::new(),
            candidate: Candidate {
                id: "test-fixture".into(),
                tool: "ffmpeg".into(),
                channel: "stable".into(),
                version: "test".into(),
                platform: "test".into(),
                notes_url: String::new(),
                source: String::new(),
                license: String::new(),
                artifacts: vec![],
                python_abi: None,
            },
        }],
        BTreeMap::from([("ffmpeg".into(), "test-ffmpeg".into())]),
    );
}
