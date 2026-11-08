//! Music discovery for requests: artist and album pages that join Lidarr with
//! MusicBrainz, ListenBrainz charts and similar artists, and artwork cached on disk.
use super::*;
use axum::{
    extract::Query,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use thelxinoe_core::Role;

#[cfg(test)]
#[path = "music_tests.rs"]
mod tests;

const LISTENBRAINZ: &str = "https://api.listenbrainz.org/1";
const SIMILAR_ARTISTS: &str = "https://labs.api.listenbrainz.org/similar-artists/json";
const SIMILAR_ALGORITHM: &str =
    "session_based_days_7500_session_300_contribution_5_threshold_10_limit_100_filter_True_skip_30";
const MUSICBRAINZ: &str = "https://musicbrainz.org/ws/2";
const COVERS: &str = "https://coverartarchive.org/release-group";
/// Lidarr's image proxy, which serves the artist pictures its lookups name.
const ARTIST_IMAGES: &str = "https://images.lidarr.audio/";
const FEED_LENGTH: usize = 30;
const SIMILAR_LENGTH: usize = 12;
const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(86_400);
/// MusicBrainz allows each client one request a second.
const MUSICBRAINZ_GAP: Duration = Duration::from_millis(1100);
/// MusicBrainz browses at most 100 entities a request.
const BROWSE_PAGE: usize = 100;
/// Pages read of each kind of release group: every group of the most prolific artists,
/// while composers and "Various Artists", with many thousands, stop at a bounded wait.
const BROWSE_PAGES: usize = 10;
/// How long artwork that failed to download is answered with an error before trying again.
const ARTWORK_RETRY: Duration = Duration::from_secs(600);
const CACHED_DOCUMENTS: usize = 256;

pub(super) struct Runtime {
    http: reqwest::Client,
    documents: std::sync::Mutex<HashMap<String, (Instant, Value)>>,
    musicbrainz: tokio::sync::Mutex<Option<Instant>>,
    images: tokio::sync::Semaphore,
}
impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                // MusicBrainz asks each client to say how to reach its authors.
                .user_agent(concat!(
                    "Thelxinoe/",
                    env!("CARGO_PKG_VERSION"),
                    " ( https://github.com/Valtrius/thelxinoe )"
                ))
                // The archive serves covers from its own hosts.
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let archive = attempt.url().host_str().is_some_and(|host| {
                        host == "archive.org"
                            || host.ends_with(".archive.org")
                            || host == "coverartarchive.org"
                    });
                    if archive && attempt.previous().len() < 5 {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(20))
                .build()?,
            documents: Default::default(),
            musicbrainz: Default::default(),
            images: tokio::sync::Semaphore::new(8),
        })
    }
}

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/acquisition/music/covers/{id}", get(cover))
        .route("/api/v1/acquisition/music/search", get(search))
        .route("/api/v1/acquisition/music/feeds/{feed}", get(feed))
        .route("/api/v1/acquisition/music/albums/{id}", get(album_page))
        .route("/api/v1/acquisition/music/artists/{id}", get(artist_page))
        .route(
            "/api/v1/acquisition/music/artists/{id}/image",
            get(artist_image),
        )
        .route(
            "/api/v1/acquisition/music/artists/{id}/monitor",
            axum::routing::delete(unmonitor),
        )
}

/// An album as the music pages show it; `id` is its MusicBrainz release group.
pub(super) fn album(id: &str, title: &str, artist: Option<&str>, date: Option<&str>) -> Value {
    let date = date.filter(|date| year(date).is_some());
    json!({
        "external_id": id,
        "title": title,
        "artist": artist,
        "year": date.and_then(year),
        "date": date.map(|date| date.get(..10).unwrap_or(date)),
        "cover": format!("/api/v1/acquisition/music/covers/{id}"),
    })
}

fn year(date: &str) -> Option<i64> {
    date.get(..4)?
        .parse()
        .ok()
        .filter(|year| (1000..=9999).contains(year))
}

/// A MusicBrainz ID in its canonical hyphenated form, safe in paths and file names.
fn mbid(id: Option<&str>) -> Option<&str> {
    id.filter(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
}

fn today() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().or(item["name"].as_str()))
        .map(str::to_owned)
        .collect()
}

/// What a release group is called on its card: its first secondary type, else its primary one.
fn label(primary: &str, secondary: &[String]) -> String {
    secondary
        .iter()
        .find(|kind| !kind.eq_ignore_ascii_case("studio"))
        .map_or(primary, String::as_str)
        .to_owned()
}

/// The album and secondary types a Lidarr metadata profile allows.
#[derive(Clone, Default)]
struct Profile {
    name: String,
    primary: HashSet<String>,
    secondary: HashSet<String>,
}
impl Profile {
    fn parse(body: &Value) -> Self {
        let allowed = |list: &str| {
            body[list]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| row["allowed"] == true)
                .filter_map(|row| row["albumType"]["name"].as_str())
                .map(str::to_lowercase)
                .collect()
        };
        Self {
            name: body["name"].as_str().unwrap_or_default().to_owned(),
            primary: allowed("primaryAlbumTypes"),
            secondary: allowed("secondaryAlbumTypes"),
        }
    }
    /// Whether Lidarr keeps a release group of these types. Lidarr calls a group without
    /// secondary types "Studio".
    fn keeps(&self, primary: &str, secondary: &[String]) -> bool {
        self.primary.contains(&primary.to_lowercase())
            && if secondary.is_empty() {
                self.secondary.contains("studio")
            } else {
                secondary
                    .iter()
                    .any(|kind| self.secondary.contains(&kind.to_lowercase()))
            }
    }
}

/// The metadata profile Lidarr applies to `artist`, or to the artists it adds.
async fn profile(c: &Connection<'_>, s: &Service, artist: Option<&Value>) -> Result<Profile> {
    let id = artist
        .and_then(|artist| artist["metadataProfileId"].as_i64())
        .filter(|id| *id > 0)
        .or_else(|| {
            serde_json::from_value::<Defaults>(s.defaults.clone())
                .ok()
                .and_then(|defaults| defaults.metadata_profile)
        });
    let body = match id {
        Some(id) => c.get(&format!("metadataprofile/{id}")).await?,
        None => c
            .get("metadataprofile")
            .await?
            .get(0)
            .cloned()
            .unwrap_or_default(),
    };
    Ok(Profile::parse(&body))
}

/// Refuses an album that Lidarr's metadata profile would leave out.
pub(super) async fn check_profile(c: &Connection<'_>, s: &Service, row: &Value) -> Result<()> {
    if row["id"].as_i64().is_some_and(|id| id > 0) {
        return Ok(());
    }
    let artist = match mbid(row["artist"]["foreignArtistId"].as_str()) {
        Some(id) => lidarr_artist(c, id).await?,
        None => None,
    };
    let profile = profile(c, s, artist.as_ref()).await?;
    let primary = row["albumType"].as_str().unwrap_or("Other");
    let secondary = names(&row["secondaryTypes"]);
    if profile.keeps(primary, &secondary) {
        return Ok(());
    }
    Err(ApiError::conflict(format!(
        "Lidarr's {} metadata profile leaves out {} releases",
        profile.name,
        label(primary, &secondary).to_lowercase()
    )))
}

/// Lidarr's artist with this MusicBrainz ID.
pub(super) async fn lidarr_artist(c: &Connection<'_>, id: &str) -> Result<Option<Value>> {
    Ok(c.fetch::<Vec<Value>>("artist", &[("mbId", id.to_owned())])
        .await?
        .into_iter()
        .find(|artist| artist["foreignArtistId"] == id))
}

/// The artist as Lidarr's metadata server describes it, whether or not Lidarr holds it.
pub(super) async fn artist_lookup(c: &Connection<'_>, id: &str) -> Result<Option<Value>> {
    let rows = c
        .call(
            reqwest::Method::GET,
            "artist/lookup",
            &[("term", format!("lidarr:{id}"))],
            None,
        )
        .await?;
    let row = rows
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["foreignArtistId"] == id))
        .cloned();
    if let Some(row) = &row {
        remember_poster(&c.state.managers.music, row);
    }
    Ok(row)
}

/// The artist as MusicBrainz describes it, in the shape of a Lidarr lookup.
async fn musicbrainz_artist(state: &AppState, id: &str) -> Result<Value> {
    let body = remote(
        state,
        format!("{MUSICBRAINZ}/artist/{id}?inc=genres&fmt=json"),
        DAY,
        "MusicBrainz",
    )
    .await?;
    Ok(json!({
        "foreignArtistId": id,
        "artistName": body["name"],
        "artistType": body["type"],
        "disambiguation": body["disambiguation"],
        "genres": names(&body["genres"]),
    }))
}

/// An artist as the music pages show it.
fn artist_card(row: &Value) -> Option<Value> {
    let id = mbid(row["foreignArtistId"].as_str())?;
    Some(json!({
        "id": id,
        "name": row["artistName"].as_str()?,
        "type": row["artistType"].as_str().filter(|kind| !kind.is_empty()),
        "disambiguation": row["disambiguation"].as_str().filter(|text| !text.is_empty()),
        "genres": names(&row["genres"]).into_iter().take(5).collect::<Vec<_>>(),
        "image": format!("/api/v1/acquisition/music/artists/{id}/image"),
    }))
}

/// How the viewer stands with an artist: whether Lidarr holds and monitors its new
/// releases, the viewer's latest request to monitor them, and its library artist.
async fn artist_standing(
    state: &AppState,
    user: &str,
    service: &str,
    card: &mut Value,
    held: Option<&Value>,
) -> Result<()> {
    let id = card["id"].as_str().unwrap_or_default().to_owned();
    let mut viewer = [json!({ "external_id": id })];
    super::requests::decorate(state, user, service, &mut viewer).await?;
    card["in_lidarr"] = json!(held.is_some());
    card["monitored"] = json!(
        held.is_some_and(|artist| artist["monitored"] == true && artist["monitorNewItems"] == "all")
    );
    card["request"] = viewer[0]["request"].take();
    card["library_id"] = json!(super::requests::library_artist(state, &id).await?);
    Ok(())
}

/// A release group, with what decides whether Lidarr keeps it.
struct Group {
    value: Value,
    primary: String,
    secondary: Vec<String>,
}

fn from_musicbrainz(row: &Value, artist: &str, artist_id: &str) -> Option<Group> {
    let id = mbid(row["id"].as_str())?;
    let primary = row["primary-type"].as_str().unwrap_or("Other").to_owned();
    let secondary = names(&row["secondary-types"]);
    let mut value = album(
        id,
        row["title"].as_str()?,
        Some(artist),
        row["first-release-date"].as_str(),
    );
    value["artist_id"] = json!(artist_id);
    value["type"] = json!(label(&primary, &secondary));
    Some(Group {
        value,
        primary,
        secondary,
    })
}

/// A Lidarr album, credited to `artist` when the row does not name its own.
fn from_lidarr(row: &Value, artist: Option<(&str, &str)>) -> Option<Group> {
    let id = mbid(row["foreignAlbumId"].as_str())?;
    let primary = row["albumType"].as_str().unwrap_or("Other").to_owned();
    let secondary = names(&row["secondaryTypes"]);
    let (name, artist_id) = match (
        row["artist"]["artistName"].as_str(),
        row["artist"]["foreignArtistId"].as_str(),
    ) {
        (Some(name), Some(id)) => (Some(name), Some(id)),
        _ => artist.map_or((None, None), |(name, id)| (Some(name), Some(id))),
    };
    let mut value = album(
        id,
        row["title"].as_str()?,
        name,
        row["releaseDate"].as_str(),
    );
    value["artist_id"] = json!(artist_id);
    value["type"] = json!(label(&primary, &secondary));
    Some(Group {
        value,
        primary,
        secondary,
    })
}

/// What Lidarr knows of an album it holds: its track count, and whether it waits for files.
fn lidarr_facts(value: &mut Value, row: &Value, today: &str) {
    let statistics = &row["statistics"];
    if let Some(count) = statistics["totalTrackCount"].as_i64().filter(|n| *n > 0) {
        value["track_count"] = json!(count);
    }
    let released = row["releaseDate"]
        .as_str()
        .is_some_and(|date| year(date).is_some() && date <= today);
    value["missing"] = json!(
        row["monitored"] == true
            && released
            && statistics["trackFileCount"].as_i64().unwrap_or(0) == 0
    );
}

fn newest_first(items: &mut [Value]) {
    items.sort_by(|a, b| b["date"].as_str().cmp(&a["date"].as_str()));
}

/// Splits an artist's release groups into those Lidarr's profile keeps and the rest,
/// newest first. Albums Lidarr already holds are kept, with what it knows of them.
fn discography(
    groups: &[Value],
    lidarr: &[Value],
    profile: &Profile,
    artist: (&str, &str),
    today: &str,
) -> (Vec<Value>, Vec<Value>) {
    let held = lidarr
        .iter()
        .filter_map(|row| Some((row["foreignAlbumId"].as_str()?, row)))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let (mut kept, mut left) = (Vec::new(), Vec::new());
    let groups = groups
        .iter()
        .filter_map(|row| from_musicbrainz(row, artist.0, artist.1))
        .chain(
            lidarr
                .iter()
                .filter_map(|row| from_lidarr(row, Some(artist))),
        );
    for mut group in groups {
        let id = group.value["external_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if !seen.insert(id.clone()) {
            continue;
        }
        let row = held.get(id.as_str());
        if let Some(row) = row {
            lidarr_facts(&mut group.value, row, today);
        }
        let keep = row.is_some() || profile.keeps(&group.primary, &group.secondary);
        group.value["requestable"] = json!(keep);
        if keep {
            kept.push(group.value);
        } else {
            left.push(group.value);
        }
    }
    newest_first(&mut kept);
    newest_first(&mut left);
    (kept, left)
}

/// The artist's release groups as MusicBrainz lists them by default: albums, then the rest.
async fn release_groups(state: &AppState, id: &str) -> Result<Vec<Value>> {
    let mut groups = Vec::new();
    for types in ["album", "single|ep|broadcast|other"] {
        let mut offset = Some(0);
        for _ in 0..BROWSE_PAGES {
            let Some(from) = offset else { break };
            let body = remote(
                state,
                format!(
                    "{MUSICBRAINZ}/release-group?artist={id}&type={types}&release-group-status=website-default&limit={BROWSE_PAGE}&offset={from}&fmt=json"
                ),
                DAY,
                "MusicBrainz",
            )
            .await?;
            offset = next_offset(&body, from);
            groups.extend(
                body["release-groups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
    }
    Ok(groups)
}

/// Where the page after the release groups `body` lists from `offset` starts, if any remain.
fn next_offset(body: &Value, offset: usize) -> Option<usize> {
    let read = offset + body["release-groups"].as_array().map_or(0, Vec::len);
    let total = body["release-group-count"]
        .as_u64()
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0);
    (read > offset && read < total).then_some(read)
}

/// An artist's discography split by `profile`, with Lidarr's albums when it holds the artist.
/// Without MusicBrainz, an artist Lidarr holds still shows Lidarr's albums.
async fn artist_albums(
    c: &Connection<'_>,
    held: Option<&Value>,
    groups: Result<Vec<Value>>,
    profile: &Profile,
    artist: (&str, &str),
) -> Result<(Vec<Value>, Vec<Value>)> {
    let lidarr = match held.and_then(|artist| artist["id"].as_i64()) {
        Some(id) => {
            c.fetch::<Vec<Value>>("album", &[("artistId", id.to_string())])
                .await?
        }
        None => Vec::new(),
    };
    let groups = match groups {
        Ok(groups) => groups,
        Err(_) if held.is_some() => Vec::new(),
        Err(error) => return Err(error),
    };
    Ok(discography(&groups, &lidarr, profile, artist, &today()))
}

#[derive(Deserialize)]
struct Scope {
    service_id: String,
}

async fn lidarr(state: &AppState, id: &str) -> Result<Service> {
    let s = service(state, id).await?;
    if s.kind != "lidarr" {
        return Err(ApiError::bad("Choose a Lidarr service"));
    }
    Ok(s)
}

async fn artist_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<Scope>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let id = mbid(Some(&id))
        .ok_or_else(|| ApiError::bad("Invalid artist identifier"))?
        .to_owned();
    let s = lidarr(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let (held, groups) = tokio::join!(lidarr_artist(&c, &id), release_groups(&state, &id));
    let held = held?;
    let row = match &held {
        Some(row) => row.clone(),
        None => match artist_lookup(&c, &id).await {
            Ok(Some(row)) => row,
            _ => musicbrainz_artist(&state, &id).await?,
        },
    };
    let mut card = artist_card(&row).ok_or_else(ApiError::not_found)?;
    card["overview"] = json!(row["overview"].as_str().filter(|text| !text.is_empty()));
    let name = card["name"].as_str().unwrap_or_default().to_owned();
    let profile = profile(&c, &s, held.as_ref()).await?;
    let (mut albums, mut other) =
        artist_albums(&c, held.as_ref(), groups, &profile, (&name, &id)).await?;
    super::requests::decorate(&state, &p.user.id, &s.id, &mut albums).await?;
    super::requests::decorate(&state, &p.user.id, &s.id, &mut other).await?;
    artist_standing(&state, &p.user.id, &s.id, &mut card, held.as_ref()).await?;
    Ok(Json(json!({
        "artist": card,
        "albums": albums,
        "other": other,
        "profile": profile.name,
    })))
}

/// Whether two artist names read the same, ignoring case, spaces and punctuation.
fn same_name(a: &str, b: &str) -> bool {
    let fold = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let a = fold(a);
    !a.is_empty() && a == fold(b)
}

#[derive(Deserialize)]
struct Search {
    service_id: String,
    term: String,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<Search>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let term = input.term.trim();
    if term.chars().count() < 2 || term.len() > 200 {
        return Err(ApiError::bad("Enter between 2 and 200 characters"));
    }
    let s = lidarr(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let query = [("term", term.to_owned())];
    let (artists, albums, standard) = tokio::join!(
        c.call(reqwest::Method::GET, "artist/lookup", &query, None),
        c.call(reqwest::Method::GET, "album/lookup", &query, None),
        profile(&c, &s, None),
    );
    let (artists, albums, standard) = (artists?, albums?, standard?);
    let music = &state.managers.music;
    let mut cards = Vec::new();
    for row in artists.as_array().into_iter().flatten() {
        if let Some(card) = artist_card(row) {
            remember_poster(music, row);
            cards.push(card);
        }
        if cards.len() == 10 {
            break;
        }
    }
    let top = cards
        .first()
        .filter(|card| {
            card["name"]
                .as_str()
                .is_some_and(|name| same_name(name, term))
        })
        .cloned();
    let mut discography = Vec::new();
    let top = match top {
        Some(mut card) => {
            cards.remove(0);
            let id = card["id"].as_str().unwrap_or_default().to_owned();
            let name = card["name"].as_str().unwrap_or_default().to_owned();
            let (held, groups) = tokio::join!(lidarr_artist(&c, &id), release_groups(&state, &id));
            let held = held?;
            let profile = match &held {
                Some(artist) => profile(&c, &s, Some(artist)).await?,
                None => standard.clone(),
            };
            // The artists and other matches still show while MusicBrainz is unavailable.
            (discography, _) = artist_albums(&c, held.as_ref(), groups, &profile, (&name, &id))
                .await
                .unwrap_or_default();
            card["album_count"] = json!(discography.len());
            artist_standing(&state, &p.user.id, &s.id, &mut card, held.as_ref()).await?;
            Some(card)
        }
        None => None,
    };
    let top_id = top.as_ref().and_then(|card| card["id"].as_str());
    let (mut matches, mut excluded) = (Vec::new(), Vec::new());
    for row in albums.as_array().into_iter().flatten().take(50) {
        if top_id.is_some_and(|id| row["artist"]["foreignArtistId"] == id) {
            continue;
        }
        let Some(mut group) = from_lidarr(row, None) else {
            continue;
        };
        let keep = row["id"].as_i64().is_some_and(|id| id > 0)
            || standard.keeps(&group.primary, &group.secondary);
        group.value["requestable"] = json!(keep);
        if keep {
            matches.push(group.value);
        } else {
            excluded.push(group.value);
        }
    }
    for items in [&mut discography, &mut matches, &mut excluded] {
        super::requests::decorate(&state, &p.user.id, &s.id, items).await?;
    }
    Ok(Json(json!({
        "artist": top,
        "artists": cards,
        "albums": discography,
        "matches": matches,
        "excluded": excluded,
    })))
}

/// A release's tracks, numbered within each disc.
fn tracklist(release: &Value) -> Vec<Value> {
    release["media"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|medium| {
            let disc = medium["position"].as_i64().unwrap_or(1);
            medium["tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |track| {
                    Some(json!({
                        "disc": disc,
                        "number": track["position"].as_i64()?,
                        "title": track["title"].as_str()?,
                        "length": track["length"].as_i64().map(|ms| ms / 1000),
                    }))
                })
        })
        .collect()
}

/// The album with its edition and tracks, from the release Lidarr monitors.
fn album_facts(value: &mut Value, row: &Value, tracks: Vec<Value>) {
    let releases = row["releases"].as_array();
    let release = releases
        .and_then(|releases| {
            releases
                .iter()
                .find(|release| release["monitored"] == true)
                .or(releases.first())
        })
        .cloned()
        .unwrap_or_default();
    let text = |value: &Value| {
        value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    value["label"] = json!(release["label"].get(0).and_then(text));
    value["format"] = json!(text(&release["format"]));
    value["edition"] = json!(text(&release["disambiguation"]));
    let duration = release["duration"]
        .as_i64()
        .or(row["duration"].as_i64())
        .filter(|ms| *ms > 0);
    value["duration"] = json!(duration.map(|ms| ms / 1000));
    let count = release["trackCount"]
        .as_i64()
        .filter(|n| *n > 0)
        .or_else(|| value["track_count"].as_i64())
        .or_else(|| (!tracks.is_empty()).then_some(tracks.len() as i64));
    value["track_count"] = json!(count);
    value["tracks"] = json!(tracks);
}

async fn album_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<Scope>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let id = mbid(Some(&id))
        .ok_or_else(|| ApiError::bad("Invalid album identifier"))?
        .to_owned();
    let s = lidarr(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let held = c
        .fetch::<Vec<Value>>("album", &[("foreignAlbumId", id.clone())])
        .await?
        .into_iter()
        .find(|row| row["foreignAlbumId"] == id);
    let mut row = match &held {
        Some(row) => row.clone(),
        None => super::requests::lookup(&c, &id).await?,
    };
    if !row["artist"].is_object()
        && let Some(artist) = row["artistId"].as_i64().filter(|id| *id > 0)
    {
        row["artist"] = c.get(&format!("artist/{artist}")).await?;
    }
    let mut group = from_lidarr(&row, None).ok_or_else(unavailable)?;
    let requestable = held.is_some() || {
        let artist = match mbid(row["artist"]["foreignArtistId"].as_str()) {
            Some(artist) => lidarr_artist(&c, artist).await?,
            None => None,
        };
        profile(&c, &s, artist.as_ref())
            .await?
            .keeps(&group.primary, &group.secondary)
    };
    if let Some(held) = &held {
        lidarr_facts(&mut group.value, held, &today());
    }
    group.value["requestable"] = json!(requestable);
    let release = row["releases"].as_array().and_then(|releases| {
        releases
            .iter()
            .find(|release| release["monitored"] == true)
            .or(releases.first())
    });
    // The album still opens without its tracks while MusicBrainz is unavailable.
    let tracks = match release.and_then(|release| mbid(release["foreignReleaseId"].as_str())) {
        Some(release) => remote(
            &state,
            format!("{MUSICBRAINZ}/release/{release}?inc=recordings&fmt=json"),
            DAY,
            "MusicBrainz",
        )
        .await
        .map(|body| tracklist(&body))
        .unwrap_or_default(),
        None => Vec::new(),
    };
    album_facts(&mut group.value, &row, tracks);
    let mut items = [group.value];
    super::requests::decorate(&state, &p.user.id, &s.id, &mut items).await?;
    let [value] = items;
    Ok(Json(value))
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HeldArtist {
    foreign_artist_id: String,
    artist_name: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Statistics {
    track_file_count: i64,
    total_track_count: i64,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HeldAlbum {
    foreign_album_id: String,
    title: String,
    release_date: Option<String>,
    monitored: bool,
    artist: Option<HeldArtist>,
    statistics: Option<Statistics>,
}

/// Released albums of Lidarr's artists without files, newest first.
fn missing(rows: Vec<HeldAlbum>, today: &str) -> Vec<Value> {
    let mut rows = rows
        .into_iter()
        .filter(|row| {
            row.release_date
                .as_deref()
                .is_some_and(|date| year(date).is_some() && date <= today)
                && row
                    .statistics
                    .as_ref()
                    .is_none_or(|statistics| statistics.track_file_count == 0)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.release_date.cmp(&a.release_date));
    rows.into_iter()
        .filter_map(|row| {
            let id = mbid(Some(&row.foreign_album_id))?;
            let artist = row.artist.as_ref();
            let mut value = album(
                id,
                &row.title,
                artist.map(|artist| artist.artist_name.as_str()),
                row.release_date.as_deref(),
            );
            value["artist_id"] = json!(artist.map(|artist| &artist.foreign_artist_id));
            value["missing"] = json!(row.monitored);
            value["requestable"] = json!(true);
            if let Some(count) = row
                .statistics
                .as_ref()
                .map(|statistics| statistics.total_track_count)
                .filter(|n| *n > 0)
            {
                value["track_count"] = json!(count);
            }
            Some(value)
        })
        .take(200)
        .collect()
}

/// The week's most listened release groups. A group appears once per release.
fn trending(body: &Value) -> Vec<Value> {
    let mut seen = HashSet::new();
    body["payload"]["release_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = mbid(row["release_group_mbid"].as_str())?;
            let mut value = album(
                id,
                row["release_group_name"].as_str()?,
                row["artist_name"].as_str(),
                None,
            );
            value["artist_id"] = json!(row["artist_mbids"].get(0).and_then(|id| mbid(id.as_str())));
            Some(value)
        })
        .filter(|album| seen.insert(album["external_id"].to_string()))
        .take(FEED_LENGTH)
        .collect()
}

/// Artists like the seed that Lidarr does not hold, closest first.
fn similar_artists(body: &Value, held: &HashSet<&str>) -> Vec<String> {
    body.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| mbid(row["artist_mbid"].as_str()))
        .filter(|id| !held.contains(id))
        .take(SIMILAR_LENGTH)
        .map(str::to_owned)
        .collect()
}

/// An artist's most listened album.
fn top_album(body: &Value) -> Option<Value> {
    let row = body
        .as_array()?
        .iter()
        .find(|row| row["release_group"]["type"] == "Album")?;
    let group = &row["release_group"];
    let mut value = album(
        mbid(row["release_group_mbid"].as_str())?,
        group["name"].as_str()?,
        row["artist"]["name"].as_str(),
        group["date"].as_str(),
    );
    value["artist_id"] = json!(
        row["artist"]["artists"][0]["artist_mbid"]
            .as_str()
            .and_then(|id| mbid(Some(id)))
    );
    Some(value)
}

/// Albums by artists like one in the library, which changes each day. Artists the library
/// or Lidarr already has are left out.
async fn similar(state: &AppState, c: &Connection<'_>) -> Result<(Vec<Value>, Value)> {
    let artists = super::requests::library_artists(state)
        .await?
        .into_iter()
        .filter(|(_, id)| mbid(Some(id)).is_some())
        .collect::<Vec<_>>();
    if artists.is_empty() {
        return Ok((Vec::new(), Value::Null));
    }
    let day = chrono::Utc::now().timestamp().div_euclid(86_400) as usize;
    let (name, id) = &artists[day % artists.len()];
    let lidarr = c.fetch::<Vec<HeldArtist>>("artist", &[]).await?;
    let held = artists
        .iter()
        .map(|(_, id)| id.as_str())
        .chain(
            lidarr
                .iter()
                .map(|artist| artist.foreign_artist_id.as_str()),
        )
        .collect::<HashSet<_>>();
    let body = remote(
        state,
        format!("{SIMILAR_ARTISTS}?artist_mbids={id}&algorithm={SIMILAR_ALGORITHM}"),
        DAY,
        "ListenBrainz",
    )
    .await?;
    let tops = futures_util::future::join_all(similar_artists(&body, &held).iter().map(|id| {
        remote(
            state,
            format!("{LISTENBRAINZ}/popularity/top-release-groups-for-artist/{id}"),
            DAY,
            "ListenBrainz",
        )
    }))
    .await;
    let items = tops
        .iter()
        .filter_map(|body| top_album(body.as_ref().ok()?))
        .collect();
    Ok((items, json!({"id": id, "name": name})))
}

async fn feed(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(input): Query<Scope>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    let s = lidarr(&state, &input.service_id).await?;
    let (mut items, artist) = match name.as_str() {
        "missing" => {
            let c = Connection::open(&state, &s).await?;
            let rows = c.fetch::<Vec<HeldAlbum>>("album", &[]).await?;
            (missing(rows, &today()), Value::Null)
        }
        "similar" => {
            let c = Connection::open(&state, &s).await?;
            similar(&state, &c).await?
        }
        "trending" => {
            let body = remote(
                &state,
                format!("{LISTENBRAINZ}/stats/sitewide/release-groups?count=100&range=week"),
                HOUR,
                "ListenBrainz",
            )
            .await?;
            (trending(&body), Value::Null)
        }
        _ => return Err(ApiError::not_found()),
    };
    super::requests::decorate(&state, &p.user.id, &s.id, &mut items).await?;
    if name == "missing" {
        items.retain(|item| item["in_library"] != true);
        items.truncate(FEED_LENGTH);
    }
    Ok(Json(json!({ "items": items, "artist": artist })))
}

async fn unmonitor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<Scope>,
) -> Result<Json<Value>> {
    let p = security::principal(&state, &headers).await?;
    if p.user.role != Role::Admin {
        return Err(ApiError::forbidden());
    }
    let id = mbid(Some(&id)).ok_or_else(|| ApiError::bad("Invalid artist identifier"))?;
    let s = lidarr(&state, &input.service_id).await?;
    let c = Connection::open(&state, &s).await?;
    let mut artist = lidarr_artist(&c, id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let manager = artist["id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or_else(unavailable)?;
    artist["monitorNewItems"] = json!("none");
    c.call(
        reqwest::Method::PUT,
        &format!("artist/{manager}"),
        &[],
        Some(artist),
    )
    .await?;
    Ok(Json(json!({ "saved": true })))
}

fn dependency(name: &str) -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "dependency_unavailable",
        format!("{name} is unavailable. Try again later."),
    )
}

fn recall(music: &Runtime, key: &str, ttl: Duration) -> Option<Value> {
    let documents = music.documents.lock().unwrap();
    documents
        .get(key)
        .filter(|(at, _)| at.elapsed() < ttl)
        .map(|(_, value)| value.clone())
}

fn remember(music: &Runtime, key: String, value: Value) {
    let mut documents = music.documents.lock().unwrap();
    if documents.len() >= CACHED_DOCUMENTS
        && !documents.contains_key(&key)
        && let Some(oldest) = documents
            .iter()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(key, _)| key.clone())
    {
        documents.remove(&oldest);
    }
    documents.insert(key, (Instant::now(), value));
}

/// A JSON document from a public music service, kept for `ttl`. A stale copy stands in
/// while the service is unavailable.
async fn remote(state: &AppState, url: String, ttl: Duration, name: &str) -> Result<Value> {
    let music = &state.managers.music;
    if let Some(value) = recall(music, &url, ttl) {
        return Ok(value);
    }
    match fetch_json(music, &url, name).await {
        Ok(value) => {
            remember(music, url, value.clone());
            Ok(value)
        }
        Err(error) => recall(music, &url, Duration::MAX).ok_or(error),
    }
}

async fn fetch_json(music: &Runtime, url: &str, name: &str) -> Result<Value> {
    if url.starts_with(MUSICBRAINZ) {
        let mut last = music.musicbrainz.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep(MUSICBRAINZ_GAP.saturating_sub(at.elapsed())).await;
        }
        *last = Some(Instant::now());
    }
    let response = music
        .http
        .get(url)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| dependency(name))?;
    // Statistics that are still being computed come back empty.
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::not_found());
    }
    if !response.status().is_success() {
        return Err(dependency(name));
    }
    let body = bounded(response, 32 * 1024 * 1024).await?;
    serde_json::from_slice(&body).map_err(|_| dependency(name))
}

fn remember_poster(music: &Runtime, row: &Value) {
    if let (Some(id), Some(url)) = (mbid(row["foreignArtistId"].as_str()), poster(row)) {
        remember(music, format!("poster:{id}"), json!(url));
    }
}

/// The artist picture Lidarr's lookup names, served by Lidarr's image proxy.
fn poster(row: &Value) -> Option<String> {
    row["images"]
        .as_array()?
        .iter()
        .find(|image| image["coverType"] == "poster")?["remoteUrl"]
        .as_str()
        .filter(|url| url.starts_with(ARTIST_IMAGES))
        .map(str::to_owned)
}

fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn artwork_unavailable() -> ApiError {
    dependency("Artwork")
}

/// The image at `url`, or None when its service has none. A server or network failure is
/// tried once more.
async fn download(music: &Runtime, url: &str) -> Result<Option<Vec<u8>>> {
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let retry = match music.http.get(url).send().await {
            Ok(response) if response.status() == StatusCode::NOT_FOUND => return Ok(None),
            Ok(response) if response.status().is_success() => {
                let bytes = bounded(response, 10 * 1024 * 1024).await?;
                return Ok(image_kind(&bytes).is_some().then_some(bytes));
            }
            Ok(response) => response.status().is_server_error(),
            Err(_) => true,
        };
        if !retry {
            break;
        }
    }
    Err(artwork_unavailable())
}

fn served(bytes: Vec<u8>) -> Result<Response> {
    let kind = image_kind(&bytes).ok_or_else(artwork_unavailable)?;
    Ok((
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "private, max-age=604800"),
        ],
        bytes,
    )
        .into_response())
}

/// Whether the marker at `path` was written less than `ttl` ago.
async fn recent(path: &std::path::Path, ttl: Duration) -> bool {
    tokio::fs::metadata(path)
        .await
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|at| at.elapsed().is_ok_and(|age| age < ttl))
}

/// The answer a recent attempt to fetch an image left behind its `miss` and `failed` markers.
async fn remembered(miss: &std::path::Path, failed: &std::path::Path) -> Option<ApiError> {
    if recent(miss, DAY).await {
        Some(ApiError::not_found())
    } else if recent(failed, ARTWORK_RETRY).await {
        Some(artwork_unavailable())
    } else {
        None
    }
}

/// Serves `name` from the music artwork cache, downloading it from the URL `source` names
/// the first time. An image its service lacks is not asked for again for a day, and one
/// that failed to download for a few minutes.
async fn artwork(
    state: &AppState,
    name: &str,
    width: Option<u32>,
    source: impl Future<Output = Result<Option<String>>>,
) -> Result<Response> {
    let dir = state.config.cache.join("music");
    let path = dir.join(name);
    if let Some(bytes) = cached(&path, width).await {
        return served(bytes);
    }
    let miss = dir.join(format!("{name}.miss"));
    let failed = dir.join(format!("{name}.failed"));
    if let Some(error) = remembered(&miss, &failed).await {
        return Err(error);
    }
    let music = &state.managers.music;
    let _slot = music
        .images
        .acquire()
        .await
        .map_err(|_| artwork_unavailable())?;
    // Another viewer may have fetched it, or failed to, meanwhile.
    if let Some(bytes) = cached(&path, width).await {
        return served(bytes);
    }
    if let Some(error) = remembered(&miss, &failed).await {
        return Err(error);
    }
    let fetched = async {
        match source.await? {
            Some(url) => download(music, &url).await,
            None => Ok(None),
        }
    }
    .await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(anyhow::Error::from)?;
    let bytes = match fetched {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            let _ = tokio::fs::write(&miss, b"").await;
            return Err(ApiError::not_found());
        }
        Err(error) => {
            let _ = tokio::fs::write(&failed, b"").await;
            return Err(error);
        }
    };
    if let Err(error) = super::metadata::store_artwork(&dir, name, bytes.clone()).await {
        tracing::warn!(%error, "Music artwork could not be cached");
        return served(bytes);
    }
    let _ = tokio::fs::remove_file(&miss).await;
    let _ = tokio::fs::remove_file(&failed).await;
    served(cached(&path, width).await.unwrap_or(bytes))
}

/// The cached image at `path`, scaled to `width` when one is given.
async fn cached(path: &std::path::Path, width: Option<u32>) -> Option<Vec<u8>> {
    tokio::fs::read(crate::artwork::card(path.to_owned(), width).await)
        .await
        .ok()
}

async fn cover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    security::principal(&state, &headers).await?;
    let id = mbid(Some(&id)).ok_or_else(|| ApiError::bad("Invalid album identifier"))?;
    artwork(&state, &format!("cover-{id}"), None, async {
        Ok(Some(format!("{COVERS}/{id}/front-250")))
    })
    .await
}

async fn artist_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    security::principal(&state, &headers).await?;
    let id = mbid(Some(&id)).ok_or_else(|| ApiError::bad("Invalid artist identifier"))?;
    let source = async {
        if let Some(url) = recall(&state.managers.music, &format!("poster:{id}"), DAY) {
            return Ok(url.as_str().map(str::to_owned));
        }
        let Some(s) = service_for_kind(&state, "lidarr").await? else {
            return Ok(None);
        };
        let c = Connection::open(&state, &s).await?;
        Ok(artist_lookup(&c, id).await?.as_ref().and_then(poster))
    };
    // Artist pictures are large photographs; pages show them at most a few hundred pixels wide.
    artwork(&state, &format!("artist-{id}"), Some(500), source).await
}
