use super::models::ToolId;

/// Application-owned installation rules, never supplied by a remote feed.
pub struct Provider {
    pub repository: &'static str,
    pub asset_pattern: &'static str,
    pub entry: &'static str,
    pub license: &'static str,
    pub dependencies: &'static [ToolId],
}

pub fn provider(tool: ToolId) -> Provider {
    let (repository, asset_pattern, entry, license, dependencies): (_, _, _, _, &[_]) = match tool {
        ToolId::Mpv => (
            "shinchiro/mpv-winbuild-cmake",
            r"^mpv-x86_64-\d+-git-[\da-f]+\.7z$",
            "mpv.exe",
            "GPL; see upstream build and component notices",
            &[],
        ),
        ToolId::Ytdlp => (
            "yt-dlp/yt-dlp",
            r"^yt-dlp\.exe$",
            "yt-dlp.exe",
            "GPL-3.0-or-later (standalone binary)",
            &[ToolId::Deno],
        ),
        ToolId::Streamlink => (
            "streamlink/windows-builds",
            r"^streamlink-[\d.]+-\d+-py\d+-x86_64\.zip$",
            "streamlink.exe",
            "BSD-2-Clause; bundled dependencies have separate licenses",
            &[],
        ),
        ToolId::Ffmpeg => (
            "BtbN/FFmpeg-Builds",
            r"^ffmpeg-n[\d.]+-[\w-]+-win64-lgpl-([\d.]+)\.zip$",
            "ffmpeg.exe",
            "LGPL-2.1-or-later; see included notices",
            &[],
        ),
        ToolId::Deno => (
            "denoland/deno",
            r"^deno-x86_64-pc-windows-msvc\.zip$",
            "deno.exe",
            "MIT; see upstream third-party notices",
            &[],
        ),
        ToolId::Uosc => (
            "tomasklaen/uosc",
            r"^uosc\.zip$",
            "scripts/uosc/main.lua",
            "LGPL-2.1-or-later",
            &[ToolId::Mpv],
        ),
        ToolId::Thumbfast => ("po5/thumbfast", "", "thumbfast.lua", "MIT", &[ToolId::Mpv]),
        ToolId::SubSelect => (
            "CogentRedTester/mpv-sub-select",
            "",
            "sub-select.lua",
            "MIT",
            &[ToolId::Mpv],
        ),
    };
    Provider {
        repository,
        asset_pattern,
        entry,
        license,
        dependencies,
    }
}

impl Provider {
    pub fn uses_commits(&self) -> bool {
        self.asset_pattern.is_empty()
    }
}

#[cfg(test)]
pub(super) fn test_package(tool: ToolId) -> super::models::Package {
    let definition = provider(tool);
    let name = match tool {
        ToolId::Mpv => "mpv-x86_64-20260101-git-abcd.7z",
        ToolId::Ytdlp => "yt-dlp.exe",
        ToolId::Streamlink => "streamlink-8.0.0-1-py314-x86_64.zip",
        ToolId::Ffmpeg => "ffmpeg-n9.0-1-gabcd-win64-lgpl-9.0.zip",
        ToolId::Deno => "deno-x86_64-pc-windows-msvc.zip",
        ToolId::Uosc => "uosc.zip",
        ToolId::Thumbfast | ToolId::SubSelect => "snapshot.zip",
    };
    let homepage = format!("https://github.com/{}", definition.repository);
    super::models::Package {
        id: format!("{}-test", tool.key()),
        tool,
        version: "test".into(),
        channel: "recommended".into(),
        recommended: true,
        url: if definition.uses_commits() {
            format!(
                "https://codeload.github.com/{}/zip/{}",
                definition.repository,
                "a".repeat(40)
            )
        } else {
            format!("{homepage}/releases/download/test/{name}")
        },
        sha256: "a".repeat(64),
        size: 123,
        format: if name.ends_with(".7z") {
            "7z"
        } else if name.ends_with(".zip") {
            "zip"
        } else {
            "file"
        }
        .into(),
        entry: definition.entry.into(),
        provider: definition.repository.into(),
        source_url: format!("{homepage}/tree/test"),
        homepage,
        license: definition.license.into(),
        published_at: "2026-01-01T00:00:00Z".into(),
        dependencies: definition.dependencies.to_vec(),
    }
}
