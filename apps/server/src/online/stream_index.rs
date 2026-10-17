//! YouTube's fragmented MP4 files index their keyframe-aligned fragments in a
//! `sidx` box after the initialization segment. Clients play and seek them as
//! byte ranges of the original file, without conversion or probing.
use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, header};
use std::{fmt::Write, time::Duration};

/// The initialization segment and index of every tested file fit in this read.
const HEAD: u64 = 64 * 1024;
const LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct Index {
    /// Every byte before the index: the `ftyp` and `moov` boxes.
    pub init: Vec<u8>,
    pub fragments: Vec<Fragment>,
    /// The first fragment's presentation time in seconds, e.g. after B-frame delay.
    pub start: f64,
    /// Whether every fragment opens with a stream access point.
    pub independent: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Fragment {
    pub offset: u64,
    pub size: u64,
    pub duration: f64,
}
/// The provider refused the signed address, rather than the file being invalid.
#[derive(Debug)]
pub(super) struct Denied;
impl std::fmt::Display for Denied {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("The public media server denied access")
    }
}
impl std::error::Error for Denied {}

enum Parsed {
    Index(Index),
    /// The file's index ends beyond the bytes read so far.
    Need(u64),
}

fn take<'a>(bytes: &'a [u8], position: &mut usize, count: usize) -> Result<&'a [u8]> {
    let value = bytes
        .get(*position..*position + count)
        .context("Truncated media index")?;
    *position += count;
    Ok(value)
}
fn number(bytes: &[u8], position: &mut usize, count: usize) -> Result<u64> {
    Ok(take(bytes, position, count)?
        .iter()
        .fold(0, |value, byte| value << 8 | u64::from(*byte)))
}

fn parse(head: &[u8]) -> Result<Parsed> {
    let mut offset = 0u64;
    let mut moov = false;
    loop {
        ensure!(offset <= LIMIT, "The media index is missing");
        let start = offset as usize;
        if head.len() < start + 16 {
            return Ok(Parsed::Need(offset + 16));
        }
        let mut position = start;
        let mut size = number(head, &mut position, 4)?;
        let kind = take(head, &mut position, 4)?;
        if size == 1 {
            size = number(head, &mut position, 8)?;
        }
        let header = (position - start) as u64;
        ensure!(size >= header, "Invalid media box");
        ensure!(offset > 0 || kind == b"ftyp", "Not an MP4 file");
        match kind {
            b"moov" => moov = true,
            b"sidx" => {
                ensure!(moov, "The media file has no initialization segment");
                let end = offset.checked_add(size).context("Invalid media box")?;
                if (head.len() as u64) < end {
                    return Ok(Parsed::Need(end));
                }
                let ((first, fragments), earliest, independent) =
                    references(&head[start..end as usize], header)?;
                let mut anchor = end + first;
                let fragments = fragments
                    .into_iter()
                    .map(|(size, duration)| {
                        let fragment = Fragment {
                            offset: anchor,
                            size,
                            duration,
                        };
                        anchor += size;
                        fragment
                    })
                    .collect();
                return Ok(Parsed::Index(Index {
                    init: head[..start].to_vec(),
                    fragments,
                    start: earliest,
                    independent,
                }));
            }
            b"moof" | b"mdat" => bail!("The media file has no fragment index"),
            _ => {}
        }
        offset = offset.checked_add(size).context("Invalid media box")?;
    }
}
/// The first fragment's distance from the end of the index, the fragment sizes
/// and durations, the earliest presentation time, and whether each fragment
/// opens with a stream access point.
type References = ((u64, Vec<(u64, f64)>), f64, bool);
fn references(index: &[u8], header: u64) -> Result<References> {
    let mut position = header as usize;
    let version = number(index, &mut position, 1)?;
    take(index, &mut position, 3 + 4)?;
    let timescale = number(index, &mut position, 4)?;
    let width = if version == 0 { 4 } else { 8 };
    let earliest = number(index, &mut position, width)?;
    let first = number(index, &mut position, width)?;
    take(index, &mut position, 2)?;
    let count = number(index, &mut position, 2)?;
    ensure!(timescale > 0 && count > 0, "Empty media index");
    let mut fragments = Vec::with_capacity(count as usize);
    let mut independent = true;
    for _ in 0..count {
        let reference = number(index, &mut position, 4)?;
        let duration = number(index, &mut position, 4)?;
        let access = number(index, &mut position, 4)?;
        // Nested indexes reference further `sidx` boxes instead of media.
        ensure!(reference >> 31 == 0, "Nested media indexes are unsupported");
        let size = reference & 0x7fff_ffff;
        ensure!(size > 0 && duration > 0, "Invalid media fragment");
        independent &= access >> 31 == 1;
        fragments.push((size, duration as f64 / timescale as f64));
    }
    Ok((
        (first, fragments),
        earliest as f64 / timescale as f64,
        independent,
    ))
}

/// Reads a file's initialization segment and fragment index with ranged
/// requests; YouTube serves bounded ranges at full speed.
pub(super) async fn load(client: &reqwest::Client, address: &str) -> Result<Index> {
    let mut wanted = HEAD;
    loop {
        let head = tokio::time::timeout(Duration::from_secs(15), read(client, address, wanted))
            .await
            .context("The media index request timed out")??;
        match parse(&head)? {
            Parsed::Index(index) => return Ok(index),
            Parsed::Need(size) if size > wanted && (head.len() as u64) == wanted => {
                ensure!(size <= LIMIT, "The media index is too large");
                wanted = size;
            }
            Parsed::Need(_) => bail!("The media file ended before its index"),
        }
    }
}
async fn read(client: &reqwest::Client, address: &str, length: u64) -> Result<Vec<u8>> {
    let mut response = client
        .get(address)
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, format!("bytes=0-{}", length - 1))
        .send()
        .await?;
    match response.status() {
        StatusCode::PARTIAL_CONTENT | StatusCode::OK => {}
        StatusCode::FORBIDDEN => return Err(Denied.into()),
        _ => bail!("The media index request failed"),
    }
    let mut bytes = Vec::new();
    while (bytes.len() as u64) < length
        && let Some(chunk) = response.chunk().await?
    {
        bytes.extend_from_slice(&chunk);
    }
    bytes.truncate(length as usize);
    Ok(bytes)
}

impl Index {
    pub fn duration(&self) -> f64 {
        self.fragments.iter().map(|f| f.duration).sum()
    }
    /// Start times of the keyframe-aligned fragments, from zero.
    pub fn starts(&self) -> Vec<f64> {
        self.fragments
            .iter()
            .scan(0.0, |start, fragment| {
                let value = *start;
                *start += fragment.duration;
                Some(value)
            })
            .collect()
    }
    /// Peak and average bit rates of the fragments.
    pub fn bandwidth(&self) -> (u64, u64) {
        let rate = |size: u64, duration: f64| (size as f64 * 8.0 / duration.max(0.001)) as u64;
        let peak = self
            .fragments
            .iter()
            .map(|f| rate(f.size, f.duration))
            .max()
            .unwrap_or(0);
        let total = self.fragments.iter().map(|f| f.size).sum();
        (peak, rate(total, self.duration()))
    }
    /// A VOD media playlist addressing each fragment as a byte range of `media`.
    pub fn playlist(&self, media: &str, init: &str) -> String {
        let target = self
            .fragments
            .iter()
            .map(|f| f.duration.ceil() as u64)
            .max()
            .unwrap_or(1);
        let mut playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n{}#EXT-X-MAP:URI=\"{init}\"\n",
            if self.independent {
                "#EXT-X-INDEPENDENT-SEGMENTS\n"
            } else {
                ""
            }
        );
        for fragment in &self.fragments {
            let _ = write!(
                playlist,
                "#EXTINF:{:.6},\n#EXT-X-BYTERANGE:{}@{}\n{media}\n",
                fragment.duration, fragment.size, fragment.offset
            );
        }
        playlist.push_str("#EXT-X-ENDLIST\n");
        playlist
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn boxed(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes
    }
    /// A file shaped like YouTube's: `ftyp`, `moov`, `sidx`, then fragments.
    pub fn file(fragments: &[(u32, u32)], timescale: u32, gap: u32) -> Vec<u8> {
        let mut index = vec![0, 0, 0, 0, 0, 0, 0, 1];
        index.extend(timescale.to_be_bytes());
        index.extend(250u32.to_be_bytes());
        index.extend(gap.to_be_bytes());
        index.extend(0u16.to_be_bytes());
        index.extend((fragments.len() as u16).to_be_bytes());
        for (size, duration) in fragments {
            index.extend(size.to_be_bytes());
            index.extend(duration.to_be_bytes());
            index.extend((1u32 << 31 | 1 << 28).to_be_bytes());
        }
        let mut bytes = boxed(b"ftyp", b"dash\0\0\0\0");
        bytes.extend(boxed(b"moov", &[7; 24]));
        bytes.extend(boxed(b"sidx", &index));
        bytes.extend(vec![0; gap as usize]);
        for (size, _) in fragments {
            bytes.extend(boxed(b"moof", &vec![1; *size as usize - 8]));
        }
        bytes
    }

    #[test]
    fn fragments_are_byte_ranges_after_the_index() {
        let bytes = file(&[(100, 9000), (60, 4500)], 1000, 4);
        let Parsed::Index(index) = parse(&bytes).unwrap() else {
            panic!("complete index")
        };
        assert_eq!(&index.init[4..8], b"ftyp");
        assert_eq!(index.init.len(), 16 + 32);
        let first = bytes.len() as u64 - 160;
        assert_eq!(
            index.fragments,
            [
                Fragment {
                    offset: first,
                    size: 100,
                    duration: 9.0
                },
                Fragment {
                    offset: first + 100,
                    size: 60,
                    duration: 4.5
                }
            ]
        );
        assert!(index.independent);
        assert_eq!(index.start, 0.25);
        assert_eq!(index.starts(), [0.0, 9.0]);
        assert_eq!(&bytes[first as usize + 4..first as usize + 8], b"moof");
        assert_eq!(index.bandwidth(), (106, 94));
        let playlist = index.playlist("../../remote/video?grant=g", "init?grant=g");
        assert!(playlist.contains("#EXT-X-TARGETDURATION:9\n"));
        assert!(playlist.contains(&format!(
            "#EXTINF:4.500000,\n#EXT-X-BYTERANGE:60@{}\n../../remote/video?grant=g\n",
            first + 100
        )));
        assert!(playlist.ends_with("#EXT-X-ENDLIST\n"));
    }

    #[test]
    fn a_long_index_requests_its_remaining_bytes() {
        let bytes = file(&[(100, 9000); 400], 1000, 0);
        assert!(matches!(parse(&bytes[..200]).unwrap(), Parsed::Need(n) if n > 200));
        assert!(matches!(parse(&bytes[..20]).unwrap(), Parsed::Need(_)));
    }

    #[test]
    fn files_without_an_index_or_initialization_are_rejected() {
        let mut fragmented = boxed(b"ftyp", b"dash");
        fragmented.extend(boxed(b"moov", &[0; 16]));
        fragmented.extend(boxed(b"moof", &[0; 16]));
        let mut nested = file(&[(100, 9000)], 1000, 0);
        // Mark the only reference as another index.
        let reference = 16 + 32 + 8 + 24;
        nested[reference] |= 0x80;
        for bytes in [
            fragmented,
            boxed(b"moov", &[0; 16]),
            [boxed(b"ftyp", b"dash"), boxed(b"sidx", &[0; 24])].concat(),
            nested,
            [
                boxed(b"ftyp", b"dash"),
                vec![0, 0, 0, 4],
                b"free".to_vec(),
                vec![0; 8],
            ]
            .concat(),
        ] {
            assert!(parse(&bytes).is_err());
        }
    }
}
