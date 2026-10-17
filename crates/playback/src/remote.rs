use anyhow::{Result, ensure};
use std::net::Ipv4Addr;

#[derive(Debug)]
pub struct MediaAccessDenied;
impl std::fmt::Display for MediaAccessDenied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("The public media server denied access")
    }
}
impl std::error::Error for MediaAccessDenied {}

/// Public extractor addresses remain in server memory, never client responses.
#[derive(Clone)]
pub struct RemoteSource {
    pub video: String,
    pub audio: Option<String>,
    pub live: bool,
    /// A loopback CONNECT proxy that keeps every media connection, including
    /// HLS segments, on the address family the extractor used.
    pub proxy: Option<String>,
    /// Start times of the video's keyframe-aligned fragments, from the
    /// provider's index. They replace probing the remote file for seek points.
    pub segments: Vec<f64>,
}
impl RemoteSource {
    pub async fn probe(&self, tools: &thelxinoe_tools::MediaTools) -> Result<serde_json::Value> {
        use std::{process::Stdio, time::Duration};
        use tokio::{io::AsyncReadExt, process::Command};
        self.validate()?;
        tools.ffprobe.verify_unchanged().await?;
        let mut command = Command::new(&tools.ffprobe.path);
        command.env_remove("FFREPORT").args(self.network_args()).args([
            "-v", "error", "-rw_timeout", "15000000", "-show_entries",
            "format=duration,start_time,bit_rate:stream=index,codec_type,codec_name,profile,level,pix_fmt,width,height,avg_frame_rate,start_time,sample_rate,channels,bit_rate,has_b_frames,color_transfer,field_order,sample_aspect_ratio",
            "-of", "json", &self.video,
        ]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn()?;
        let mut bytes = Vec::new();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing media probe output"))?
            .take(65537);
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("Missing media probe diagnostics"))?;
        let mut diagnostics = Vec::new();
        let read_diagnostics = async {
            let mut buffer = [0; 1024];
            loop {
                let count = stderr.read(&mut buffer).await?;
                if count == 0 {
                    break;
                }
                // Drain all stderr so a full pipe cannot stall the probe.
                let keep = count.min(8192 - diagnostics.len());
                diagnostics.extend_from_slice(&buffer[..keep]);
            }
            Ok::<_, std::io::Error>(())
        };
        tokio::time::timeout(Duration::from_secs(20), async {
            tokio::try_join!(stdout.read_to_end(&mut bytes), read_diagnostics)
        })
        .await??;
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait()).await??;
        if !status.success() && String::from_utf8_lossy(&diagnostics).contains("403 Forbidden") {
            return Err(MediaAccessDenied.into());
        }
        ensure!(
            bytes.len() <= 65536 && status.success(),
            "Could not inspect public media codecs"
        );
        let mut probe: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(audio) = &self.audio {
            let source = Self {
                video: audio.clone(),
                audio: None,
                ..self.clone()
            };
            // Separate inputs have their own stream indexes; normalize them to the combined source.
            let other = Box::pin(source.probe(tools)).await?;
            let streams = probe["streams"]
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("Missing media streams"))?;
            for mut stream in other["streams"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["codec_type"] == "audio")
                .cloned()
            {
                stream["index"] = serde_json::json!(streams.len());
                streams.push(stream);
            }
        }
        Ok(probe)
    }
    /// Network input options for each ffmpeg or ffprobe remote address.
    pub fn network_args(&self) -> Vec<&str> {
        match &self.proxy {
            Some(proxy) => vec![
                "-protocol_whitelist",
                "https,tls,tcp,crypto,httpproxy",
                "-http_proxy",
                proxy,
            ],
            None => vec!["-protocol_whitelist", "https,tls,tcp,crypto"],
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(proxy) = &self.proxy {
            let parsed = url::Url::parse(proxy)?;
            ensure!(
                parsed.scheme() == "http"
                    && parsed.host() == Some(url::Host::Ipv4(Ipv4Addr::LOCALHOST))
                    && parsed.port().is_some()
                    && parsed.path() == "/"
                    && parsed.query().is_none()
                    && parsed.username().is_empty()
                    && parsed.password().is_none(),
                "Unsupported media proxy"
            );
        }
        for address in std::iter::once(&self.video).chain(self.audio.iter()) {
            let parsed = url::Url::parse(address)?;
            ensure!(
                address.len() <= 16384
                    && parsed.scheme() == "https"
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.port().is_none_or(|p| p == 443)
                    && parsed.host_str().is_some_and(|h| [
                        "googlevideo.com",
                        "ttvnw.net",
                        "live-video.net"
                    ]
                    .iter()
                    .any(|domain| h == *domain || h.ends_with(&format!(".{domain}")))),
                "Unsupported public media address"
            );
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_inputs_are_restricted_to_https_provider_cdn() {
        for value in [
            "file:///etc/passwd",
            "http://localhost/test",
            "https://googlevideo.com.attacker.test/video",
            "https://user:password@r1.googlevideo.com/video",
            "https://r1.googlevideo.com:444/video",
        ] {
            assert!(
                RemoteSource {
                    video: value.into(),
                    audio: None,
                    live: false,
                    proxy: None,
                    segments: Vec::new()
                }
                .validate()
                .is_err()
            );
        }
        let mut source = RemoteSource {
            video: "https://r1.googlevideo.com/videoplayback?signature=public".into(),
            audio: None,
            live: false,
            proxy: None,
            segments: Vec::new(),
        };
        assert!(source.validate().is_ok());
        assert!(!source.network_args().contains(&"-http_proxy"));
        source.proxy = Some("http://127.0.0.1:41000".into());
        assert!(source.validate().is_ok());
        assert_eq!(
            source.network_args(),
            [
                "-protocol_whitelist",
                "https,tls,tcp,crypto,httpproxy",
                "-http_proxy",
                "http://127.0.0.1:41000"
            ]
        );
        for proxy in [
            "http://192.0.2.1:41000",
            "https://127.0.0.1:41000",
            "http://127.0.0.1",
            "http://user:secret@127.0.0.1:41000",
            "http://127.0.0.1:41000/path",
        ] {
            source.proxy = Some(proxy.into());
            assert!(source.validate().is_err(), "{proxy}");
        }
    }
}
