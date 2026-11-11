#![allow(
    dead_code,
    reason = "each test crate that includes this file uses only one installer"
)]

use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
use thelxinoe_tools::{Candidate, Generation, Runtime};

const NAMES: [&str; 2] = ["ffmpeg", "ffprobe"];

// Existing media integration tests explicitly inject their runner's FFmpeg pair.
// Production has no PATH fallback.
pub fn install(runtime: &Runtime, root: &Path) {
    let directory = root.join("packages/test-ffmpeg");
    for name in NAMES {
        copy(
            name,
            &directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
        );
    }
    publish(runtime, &directory);
}

/// `install`, for fixture CDNs whose self-signed `certificate` FFmpeg would otherwise
/// refuse: the pair trusts that certificate alone.
#[cfg(unix)]
pub fn install_trusting(runtime: &Runtime, root: &Path, certificate: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let directory = root.join("packages/test-ffmpeg");
    for name in NAMES {
        let binary = directory.join(format!("{name}-binary"));
        copy(name, &binary);
        let wrapper = directory.join(name);
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nSSL_CERT_FILE='{}' exec '{}' \"$@\"\n",
                certificate.display(),
                binary.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    publish(runtime, &directory);
}

fn copy(name: &str, destination: &Path) {
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let source = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join(&filename))
        .find(|p| p.is_file())
        .expect("FFmpeg test prerequisite");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    if !destination.exists() {
        std::fs::copy(source, destination).unwrap();
    }
}

fn publish(runtime: &Runtime, directory: &Path) {
    let mut files = BTreeMap::new();
    let mut executables = BTreeMap::new();
    for name in NAMES {
        let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let hash: String = Sha256::digest(std::fs::read(directory.join(&filename)).unwrap())
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
