use anyhow::{Result, bail};
use regex::Regex;
use serde_json::Value;
use std::path::Path;

pub struct Identity {
    pub kind: String,
    pub key: String,
    pub title: String,
    pub parent: Option<(String, String)>,
    pub number: Option<i64>,
    pub year: Option<i64>,
    pub playable: bool,
}
fn item(
    kind: &str,
    key: String,
    title: String,
    parent: Option<(&str, String)>,
    number: Option<i64>,
    playable: bool,
) -> Identity {
    Identity {
        kind: kind.into(),
        key,
        title,
        parent: parent.map(|(k, v)| (k.into(), v)),
        number,
        year: None,
        playable,
    }
}
fn normalize(s: &str) -> String {
    s.replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn identify(domain: &str, path: &Path, probe: &Value) -> Result<(Vec<Identity>, String)> {
    let filename = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    let stem = normalize(filename);
    if domain == "movies" {
        let edition_pattern = Regex::new(r"(?i)\{edition-([^}]+)\}")?;
        let edition = edition_pattern
            .captures(filename)
            .map(|c| c[1].to_owned())
            .unwrap_or_default();
        let clean = edition_pattern.replace_all(&stem, "");
        let year_pattern = Regex::new(r"(?:\(|\b)((?:19|20)\d{2})(?:\)|\b)")?;
        let year = year_pattern.captures(&clean);
        let title = year
            .as_ref()
            .map(|c| clean[..c.get(0).unwrap().start()].trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| clean.trim().to_string());
        let year = year.and_then(|c| c[1].parse::<i64>().ok());
        let key = format!("{}:{}", title.to_lowercase(), year.unwrap_or(0));
        let mut movie = item("movie", key, title, None, None, true);
        movie.year = year;
        return Ok((vec![movie], edition));
    }
    if domain == "shows" {
        let pattern = Regex::new(r"(?i)S(\d{1,3})[ ._-]*E(\d{1,3})((?:[ ._-]*E\d{1,3})*)")?;
        let Some(c) = pattern.captures(filename) else {
            bail!("TV filename must contain season/episode evidence such as S01E02");
        };
        let season = c[1].parse::<i64>()?;
        let first = c[2].parse::<i64>()?;
        let extra = Regex::new(r"(?i)E(\d{1,3})")?;
        let title = normalize(&filename[..c.get(0).unwrap().start()])
            .trim_matches([' ', '-'])
            .to_string();
        let title = if title.is_empty() {
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown show")
                .to_string()
        } else {
            title
        };
        let key = title.to_lowercase();
        let season_key = format!("{key}:s{season}");
        let mut result = vec![
            item("show", key.clone(), title.clone(), None, None, false),
            item(
                "season",
                season_key.clone(),
                if season == 0 {
                    "Specials".into()
                } else {
                    format!("Season {season}")
                },
                Some(("show", key)),
                Some(season),
                false,
            ),
        ];
        let mut episodes = vec![first];
        for e in extra.captures_iter(&c[3]) {
            episodes.push(e[1].parse()?);
        }
        for number in episodes {
            result.push(item(
                "episode",
                format!("{season_key}:e{number}"),
                format!("{title} · S{season:02}E{number:02}"),
                Some(("season", season_key.clone())),
                Some(number),
                true,
            ));
        }
        return Ok((result, String::new()));
    }
    if domain == "music" {
        let tags = probe["format"]["tags"].as_object();
        let tag = |keys: &[&str]| -> Option<String> {
            tags.and_then(|t| {
                t.iter()
                    .find(|(k, _)| keys.iter().any(|key| k.eq_ignore_ascii_case(key)))
                    .and_then(|(_, v)| v.as_str())
                    .map(str::to_string)
            })
        };
        let artist =
            tag(&["album_artist", "albumartist", "artist"]).unwrap_or("Unknown artist".into());
        let album = tag(&["album"]).unwrap_or_else(|| {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown album")
                .into()
        });
        let title = tag(&["title"]).unwrap_or(stem);
        let disc = tag(&["disc", "discnumber"])
            .and_then(|s| s.split('/').next()?.parse::<i64>().ok())
            .unwrap_or(1);
        let track = tag(&["track", "tracknumber"])
            .and_then(|s| s.split('/').next()?.parse::<i64>().ok())
            .unwrap_or(0);
        let artist_id = tag(&["musicbrainz_albumartistid", "musicbrainz_artistid"]);
        let album_id = tag(&["musicbrainz_albumid"]);
        let track_id = tag(&["musicbrainz_trackid"]);
        let release_track_id = tag(&["musicbrainz_releasetrackid"]);
        let artist_key = artist_id.clone().unwrap_or_else(|| artist.to_lowercase());
        let album_key = album_id
            .clone()
            .unwrap_or_else(|| format!("{artist_key}:{}", album.to_lowercase()));
        let a = item("artist", artist_key.clone(), artist, None, None, false);
        let b = item(
            "album",
            album_key.clone(),
            album,
            Some(("artist", artist_key)),
            None,
            false,
        );
        let t = item(
            "track",
            release_track_id.clone().unwrap_or_else(|| {
                format!(
                    "{album_key}:{disc}:{track}:{}",
                    track_id.as_deref().unwrap_or(&title.to_lowercase())
                )
            }),
            title,
            Some(("album", album_key)),
            Some(disc * 10000 + track),
            true,
        );
        return Ok((vec![a, b, t], String::new()));
    }
    bail!("Unknown library domain")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recording_and_release_track_ids_do_not_merge_album_membership() {
        let probe = |album: &str| serde_json::json!({"format":{"tags":{"artist":"Artist","album":album,"track":"1","musicbrainz_trackid":"recording-id","title":"Track"}}});
        let (a, _) = identify("music", Path::new("track.flac"), &probe("First album")).unwrap();
        let (b, _) = identify("music", Path::new("track.flac"), &probe("Compilation")).unwrap();
        assert_ne!(a[2].key, b[2].key);
        let mut data = probe("First album");
        data["format"]["tags"]["musicbrainz_releasetrackid"] =
            serde_json::json!("release-track-id");
        let (c, _) = identify("music", Path::new("track.flac"), &data).unwrap();
        assert!(c[2].key.contains("release-track-id"));
    }
}
