use anyhow::{Result, ensure};
use std::net::Ipv4Addr;

/// Public extractor addresses remain in server memory, never client responses.
#[derive(Clone)]
pub struct RemoteSource {
    pub video: String,
    pub audio: Option<String>,
    pub live: bool,
    /// A loopback CONNECT proxy that keeps every media connection, including
    /// HLS segments, on the address family the extractor used.
    pub proxy: Option<String>,
    /// Start times of a remuxed HLS video's segments, each opening with a
    /// keyframe. They replace probing the remote file for seek points.
    pub segments: Vec<f64>,
}
impl RemoteSource {
    /// Where a seek to `position` starts without probing: just before the
    /// leading keyframe of its HLS segment, absorbing playlist rounding.
    pub fn segment_start(&self, position: f64) -> Option<f64> {
        let start = self
            .segments
            .iter()
            .copied()
            .take_while(|start| *start <= position)
            .last()?;
        Some((start - 0.1).max(0.0))
    }
    /// Input seek and offset that start one address's track at
    /// `timeline_start`, or `None` to read it from the beginning. Seeking to
    /// zero is not a no-op: HLS demuxing would drop the leading keyframe.
    pub fn seek(&self, video: bool, position: f64, timeline_start: f64) -> Option<(f64, f64)> {
        let seek = if video && self.segments.is_empty() {
            // Repeat the probe's original seek. Seeking to its returned
            // keyframe PTS can select the previous GOP in MP4 inputs.
            position
        } else {
            timeline_start
        };
        (seek > 0.0).then_some((seek, seek - timeline_start))
    }
    /// HLS seeks also copy packets from an earlier keyframe. Those would start
    /// video before the timeline and shift both tracks, so they are dropped.
    pub fn drops_prior_packets(&self) -> bool {
        !self.segments.is_empty()
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
    #[test]
    fn seeks_start_both_tracks_on_one_timeline() {
        let mut source = RemoteSource {
            video: "https://r1.googlevideo.com/video".into(),
            audio: Some("https://r1.googlevideo.com/audio".into()),
            live: false,
            proxy: None,
            segments: Vec::new(),
        };
        // Starting from the beginning never seeks, keeping the first keyframe.
        assert_eq!(source.seek(true, 0.0, 0.0), None);
        assert_eq!(source.seek(false, 0.0, 0.0), None);
        // Files repeat the probed position; the keyframe offset realigns video.
        assert_eq!(source.seek(true, 60.0, 54.25), Some((60.0, 5.75)));
        assert_eq!(source.seek(false, 60.0, 54.25), Some((54.25, 0.0)));
        assert!(!source.drops_prior_packets());
        source.segments = vec![0.0, 3.125, 9.5, 12.875];
        assert!(source.drops_prior_packets());
        assert_eq!(source.segment_start(0.5), Some(0.0));
        assert_eq!(source.segment_start(9.5), Some(9.4));
        assert_eq!(source.segment_start(11.0), Some(9.4));
        assert_eq!(source.segment_start(99.0), Some(12.775));
        // Both tracks seek just before the segment's leading keyframe.
        assert_eq!(source.seek(true, 11.0, 9.4), Some((9.4, 0.0)));
        assert_eq!(source.seek(false, 11.0, 9.4), Some((9.4, 0.0)));
        assert_eq!(source.seek(true, 2.0, 0.0), None);
        assert_eq!(source.seek(false, 2.0, 0.0), None);
    }
}
