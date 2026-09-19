use anyhow::{Result, ensure};

/// Public extractor addresses remain in server memory, never client responses.
#[derive(Clone)]
pub struct RemoteSource {
    pub video: String,
    pub audio: Option<String>,
    pub live: bool,
}
impl RemoteSource {
    pub fn validate(&self) -> Result<()> {
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
                    live: false
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            RemoteSource {
                video: "https://r1.googlevideo.com/videoplayback?signature=public".into(),
                audio: None,
                live: false
            }
            .validate()
            .is_ok()
        );
    }
}
