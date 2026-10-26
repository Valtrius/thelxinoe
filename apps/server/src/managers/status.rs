//! Cached Radarr and Sonarr title status: what each movie, series and episode is doing
//! upstream. It is derived and ephemeral, so it lives in memory and is rebuilt from
//! read-only polls; nothing here ever changes a manager.

#[path = "../storage/managers/status.rs"]
mod storage;
#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;

use super::*;
use axum::{body::Body, extract::Query, http::Request, response::Response};
use serde::Deserializer;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use thelxinoe_core::{Principal, Role};

/// Downloads move quickly; the queue is the only thing polled this often.
const QUEUE_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
/// Whole title lists are refreshed this often, or sooner when the queue suggests an import.
const TITLES_EVERY: std::time::Duration = std::time::Duration::from_secs(600);
const QUEUE_PAGE: usize = 250;
const QUEUE_PAGES: usize = 40;
/// A failing manager is retried less and less often, up to this interval.
const BACKOFF_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);
/// A queue that cannot be confirmed for this long stops showing progress.
const STALE_QUEUE: std::time::Duration = std::time::Duration::from_secs(300);
/// Bursts of upstream changes, such as retention deleting many titles, refresh once.
const FORCED_GAP: std::time::Duration = std::time::Duration::from_secs(10);
/// Download progress is announced in steps of this many percent.
const PROGRESS_STEP: i64 = 5;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/catalog/lookup", get(lookup))
        .route("/api/v1/catalog/{id}/seasons", get(seasons))
        .route(
            "/api/v1/catalog/titles/{service_id}/{entity_id}/artwork/{kind}",
            get(artwork),
        )
}

fn lenient<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
fn bytes<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<i64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(0, |v| v as i64))
}
fn time(value: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|at| at.timestamp())
}
/// Release dates have no time of day; noon UTC is the same date in every timezone.
pub(crate) fn day(value: Option<&str>) -> Option<i64> {
    time(value).map(|at| at - at.rem_euclid(86400) + 43200)
}
fn positive(value: Option<i64>) -> Option<i64> {
    value.filter(|v| *v > 0)
}

#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
struct Image {
    #[serde(deserialize_with = "lenient")]
    cover_type: String,
    url: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
struct SeasonFlag {
    season_number: i64,
    #[serde(deserialize_with = "lenient")]
    monitored: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
struct Statistics {
    episode_count: i64,
    episode_file_count: i64,
    total_episode_count: i64,
    #[serde(deserialize_with = "bytes")]
    size_on_disk: i64,
}
/// The TMDB rating Radarr lists, else Sonarr's own, as metadata refreshes read them.
#[derive(Clone, Copy, Debug, Default)]
struct Rating(Option<f64>);
impl Hash for Rating {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.map(f64::to_bits).hash(state);
    }
}
impl<'de> Deserialize<'de> for Rating {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let ratings = Option::<Value>::deserialize(deserializer)?.unwrap_or_default();
        Ok(Self(
            ratings
                .pointer("/tmdb/value")
                .or_else(|| ratings.pointer("/value"))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0),
        ))
    }
}
/// Whether a series' status, from Sonarr or TMDB, says no more episodes will air.
pub(crate) fn ended(status: Option<&str>) -> bool {
    status.is_some_and(|status| {
        ["ended", "canceled", "cancelled"]
            .iter()
            .any(|ended| status.eq_ignore_ascii_case(ended))
    })
}
/// One Radarr movie or Sonarr series, reduced to what status, title pages and library
/// lists show.
#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Title {
    id: i64,
    tmdb_id: Option<i64>,
    tvdb_id: Option<i64>,
    imdb_id: Option<String>,
    #[serde(deserialize_with = "lenient")]
    title: String,
    year: Option<i64>,
    overview: Option<String>,
    runtime: Option<i64>,
    #[serde(deserialize_with = "lenient")]
    genres: Vec<String>,
    ratings: Rating,
    added: Option<String>,
    #[serde(deserialize_with = "lenient")]
    monitored: bool,
    #[serde(deserialize_with = "lenient")]
    has_file: bool,
    /// Radarr's; Sonarr's is in the statistics.
    #[serde(deserialize_with = "bytes")]
    size_on_disk: i64,
    is_available: Option<bool>,
    in_cinemas: Option<String>,
    digital_release: Option<String>,
    physical_release: Option<String>,
    status: Option<String>,
    #[serde(deserialize_with = "lenient")]
    seasons: Vec<SeasonFlag>,
    statistics: Option<Statistics>,
    next_airing: Option<String>,
    previous_airing: Option<String>,
    #[serde(deserialize_with = "lenient")]
    images: Vec<Image>,
}
impl Title {
    fn tidy(mut self) -> Self {
        self.tmdb_id = positive(self.tmdb_id);
        self.tvdb_id = positive(self.tvdb_id);
        self.year = positive(self.year);
        self.runtime = positive(self.runtime);
        self.overview = self.overview.filter(|v| !v.trim().is_empty());
        self.imdb_id = self.imdb_id.filter(|v| !v.is_empty());
        self.images.retain(|image| {
            matches!(image.cover_type.as_str(), "poster" | "fanart" | "clearlogo")
                && image.url.as_deref().is_some_and(|url| url.starts_with('/'))
        });
        self
    }
    fn image(&self, kind: &str) -> Option<&str> {
        self.images
            .iter()
            .find(|image| image.cover_type == kind)
            .and_then(|image| image.url.as_deref())
    }
    /// The days a movie reaches cinemas, digital and physical release.
    fn releases(&self) -> impl Iterator<Item = i64> + '_ {
        [
            &self.in_cinemas,
            &self.digital_release,
            &self.physical_release,
        ]
        .into_iter()
        .filter_map(|date| day(date.as_deref()))
    }
    fn facts(&self) -> Facts {
        let statistics = self.statistics.as_ref();
        Facts {
            added: time(self.added.as_deref()),
            rating: self.ratings.0,
            genres: self.genres.clone(),
            size: positive(statistics.map(|s| s.size_on_disk))
                .or(positive(Some(self.size_on_disk))),
            runtime: self.runtime,
            released: self.releases().min(),
            episodes: positive(statistics.map(|s| s.episode_file_count)),
            last_aired: time(self.previous_airing.as_deref()),
            next_airing: time(self.next_airing.as_deref()),
        }
    }
    /// What decides whether a series' episode list may have changed upstream.
    fn signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            self.statistics
                .as_ref()
                .map(|s| (s.episode_count, s.episode_file_count, s.total_episode_count)),
            &self.next_airing,
            &self.previous_airing,
            self.monitored,
            self.seasons
                .iter()
                .map(|s| (s.season_number, s.monitored))
                .collect::<Vec<_>>(),
        )
            .hash(&mut hasher);
        hasher.finish()
    }
    /// The public title reference: TMDB first, TVDB only for a series without one.
    fn reference(&self, manager: &str) -> Option<String> {
        match (self.tmdb_id, self.tvdb_id) {
            (Some(tmdb), _) => Some(format!("tmdb:{tmdb}")),
            (None, Some(tvdb)) if manager == "sonarr" => Some(format!("tvdb:{tvdb}")),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Episode {
    id: i64,
    season_number: i64,
    episode_number: i64,
    title: Option<String>,
    overview: Option<String>,
    air_date_utc: Option<String>,
    #[serde(deserialize_with = "lenient")]
    monitored: bool,
    #[serde(deserialize_with = "lenient")]
    has_file: bool,
    runtime: Option<i64>,
    #[serde(skip)]
    air: Option<i64>,
}
impl Episode {
    fn tidy(mut self) -> Self {
        self.air = time(self.air_date_utc.as_deref());
        self.runtime = positive(self.runtime);
        self.overview = self.overview.filter(|v| !v.trim().is_empty());
        self
    }
    fn aired(&self, now: i64) -> bool {
        self.air.is_some_and(|at| at <= now)
    }
}
#[derive(Clone, Debug, Default, Deserialize, Hash)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct QueueItem {
    id: i64,
    movie_id: Option<i64>,
    episode_id: Option<i64>,
    status: Option<String>,
    tracked_download_state: Option<String>,
    #[serde(deserialize_with = "bytes")]
    size: i64,
    #[serde(deserialize_with = "bytes")]
    sizeleft: i64,
    estimated_completion_time: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct QueuePage {
    total_records: i64,
    #[serde(deserialize_with = "lenient")]
    records: Vec<QueueItem>,
}

/// One manager's titles, Sonarr episodes by series, and download queue. Titles and
/// episodes change only when titles are reread, so queue polls share them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Inventory {
    /// The service the titles were read from.
    service: String,
    titles: Arc<BTreeMap<i64, Title>>,
    episodes: Arc<BTreeMap<i64, Vec<Episode>>>,
    queue: Vec<QueueItem>,
    by_tmdb: Arc<HashMap<i64, i64>>,
    by_tvdb: Arc<HashMap<i64, i64>>,
    /// The titles and episodes' share of the snapshot fingerprint.
    library: u64,
}
impl Inventory {
    fn new(
        service: &str,
        titles: Vec<Title>,
        episodes: BTreeMap<i64, Vec<Episode>>,
        queue: Vec<QueueItem>,
    ) -> Self {
        let titles = titles
            .into_iter()
            .map(|t| (t.id, t))
            .collect::<BTreeMap<_, _>>();
        let (mut by_tmdb, mut by_tvdb) = (HashMap::new(), HashMap::new());
        for title in titles.values() {
            if let Some(tmdb) = title.tmdb_id {
                by_tmdb.entry(tmdb).or_insert(title.id);
            }
            if let Some(tvdb) = title.tvdb_id {
                by_tvdb.entry(tvdb).or_insert(title.id);
            }
        }
        let mut hasher = DefaultHasher::new();
        (&titles, &episodes).hash(&mut hasher);
        Self {
            service: service.to_owned(),
            titles: Arc::new(titles),
            episodes: Arc::new(episodes),
            queue,
            by_tmdb: Arc::new(by_tmdb),
            by_tvdb: Arc::new(by_tvdb),
            library: hasher.finish(),
        }
    }
}

/// The operational Radarr and Sonarr inventories by manager kind; there is at most one
/// service of each kind.
#[derive(Clone, Default)]
pub(crate) struct Snapshot {
    services: BTreeMap<String, Arc<Inventory>>,
}
impl Snapshot {
    /// The inventory read from one service.
    fn service(&self, id: &str) -> Option<&Inventory> {
        self.services
            .values()
            .map(Arc::as_ref)
            .find(|inventory| inventory.service == id)
    }
    /// Cheap change detection over what viewers can see: queue stages and progress
    /// in coarse steps, so a running download does not refresh every page each poll.
    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for (kind, inventory) in &self.services {
            let queue = inventory
                .queue
                .iter()
                .filter_map(|r| {
                    let stage = stage(r)?;
                    let step = (r.size > 0).then(|| {
                        (r.size - r.sizeleft.clamp(0, r.size)) * 100 / r.size / PROGRESS_STEP
                    });
                    Some((r.id, r.movie_id, r.episode_id, stage as u8, step))
                })
                .collect::<Vec<_>>();
            (kind, &inventory.service, inventory.library, queue).hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[derive(Default)]
pub(crate) struct Store {
    snapshot: std::sync::RwLock<Arc<Snapshot>>,
    wake: tokio::sync::Notify,
}
fn snapshot(state: &AppState) -> Arc<Snapshot> {
    state
        .managers
        .status
        .snapshot
        .read()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
}
fn replace(state: &AppState, next: Snapshot) {
    let next = Arc::new(next);
    match state.managers.status.snapshot.write() {
        Ok(mut current) => *current = next,
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}
/// Refresh every title list now, after something may have changed them upstream.
pub(crate) fn refresh_soon(state: &AppState) {
    state.managers.status.wake.notify_one();
}

#[derive(Default)]
struct Tracker {
    refreshed: Option<std::time::Instant>,
    queue: Vec<i64>,
    signatures: HashMap<i64, u64>,
    error: Option<String>,
    failures: u32,
    retry_at: Option<std::time::Instant>,
    failing_since: Option<std::time::Instant>,
}
impl Tracker {
    fn failed(&mut self) {
        let now = std::time::Instant::now();
        self.failures = self.failures.saturating_add(1);
        self.failing_since.get_or_insert(now);
        let wait = QUEUE_EVERY.saturating_mul(1 << self.failures.min(5).saturating_sub(1));
        self.retry_at = Some(now + wait.min(BACKOFF_LIMIT));
    }
    fn recovered(&mut self) {
        self.error = None;
        self.failures = 0;
        self.retry_at = None;
        self.failing_since = None;
    }
}

pub(crate) async fn run(state: AppState) -> anyhow::Result<()> {
    let mut trackers = HashMap::new();
    let mut seen = Snapshot::default().fingerprint();
    let mut force = true;
    let mut forced_at: Option<tokio::time::Instant> = None;
    let mut pruned = None;
    loop {
        if force {
            if let Some(at) = forced_at {
                tokio::time::sleep(FORCED_GAP.saturating_sub(at.elapsed())).await;
            }
            forced_at = Some(tokio::time::Instant::now());
        }
        seen = refresh(&state, &mut trackers, force, seen).await;
        // Title lists change rarely; cached artwork is checked against them when they do.
        let current = snapshot(&state);
        let key = artwork_key(&current);
        if pruned != Some(key) {
            match prune_artwork(&state, &current).await {
                Ok(removed) => {
                    if removed > 0 {
                        tracing::debug!(removed, "Removed artwork of titles no longer listed");
                    }
                    pruned = Some(key);
                }
                Err(error) => {
                    tracing::warn!(%error, "Cached title artwork could not be cleaned up");
                }
            }
        }
        force = tokio::select! {
            () = tokio::time::sleep(QUEUE_EVERY) => false,
            () = state.managers.status.wake.notified() => true,
        };
    }
}

/// Poll every service once; a failing service keeps its last inventory.
async fn refresh(
    state: &AppState,
    trackers: &mut HashMap<String, Tracker>,
    force: bool,
    seen: u64,
) -> u64 {
    let services = match storage::services(&state.db).await {
        Ok(services) => services,
        Err(error) => {
            tracing::warn!(%error, "Manager status could not read its services");
            return seen;
        }
    };
    trackers.retain(|id, _| services.iter().any(|(service, _)| service == id));
    let previous = snapshot(state);
    let mut next = Snapshot::default();
    for (id, kind) in &services {
        if let Some(current) = previous
            .services
            .get(kind)
            .filter(|inventory| &inventory.service == id)
        {
            next.services.insert(kind.clone(), current.clone());
        }
    }
    for (id, kind) in services {
        let tracker = trackers.entry(id.clone()).or_default();
        // A failing manager waits out its backoff; an explicit refresh tries it now.
        if !force
            && tracker
                .retry_at
                .is_some_and(|at| std::time::Instant::now() < at)
        {
            continue;
        }
        let current = next.services.get(&kind).cloned();
        let polled = async {
            let s = service(state, &id).await?;
            let c = Connection::open(state, &s).await?;
            poll(&c, current.as_deref(), tracker, force).await
        }
        .await;
        match polled {
            Ok(inventory) => {
                tracker.recovered();
                next.services.insert(kind, Arc::new(inventory));
            }
            Err(error) => {
                if tracker.error.as_deref() != Some(error.2.as_str()) {
                    tracing::warn!(service = %id, error = %error.2, "Keeping the last manager status");
                    tracker.error = Some(error.2);
                }
                tracker.failed();
                // Progress that can no longer be confirmed must not show forever.
                if let Some(current) = current.filter(|current| {
                    !current.queue.is_empty()
                        && tracker
                            .failing_since
                            .is_some_and(|at| at.elapsed() >= STALE_QUEUE)
                }) {
                    next.services.insert(
                        kind,
                        Arc::new(Inventory {
                            queue: Vec::new(),
                            ..(*current).clone()
                        }),
                    );
                }
            }
        }
        // Each manager's status is shown as soon as it is read.
        replace(state, next.clone());
    }
    let fingerprint = next.fingerprint();
    replace(state, next);
    if fingerprint != seen
        && let Err(error) = state.emit(None, "catalog.status.changed", json!({})).await
    {
        tracing::warn!(%error, "Could not announce a manager status change");
    }
    fingerprint
}

/// Read one manager: its queue always, its titles when due, and only the Sonarr episode
/// lists whose series changed. Tracking advances only once the whole poll succeeded.
async fn poll(
    c: &Connection<'_>,
    current: Option<&Inventory>,
    tracker: &mut Tracker,
    force: bool,
) -> Result<Inventory> {
    let queue = queue(c).await?;
    let keys = queue.iter().map(|r| r.id).collect::<Vec<_>>();
    let due = force
        || current.is_none()
        || keys != tracker.queue
        || tracker
            .refreshed
            .is_none_or(|at| at.elapsed() >= TITLES_EVERY);
    let next = match current {
        Some(current) if !due => Inventory {
            queue,
            ..current.clone()
        },
        _ => {
            let titles = c
                .fetch::<Vec<Title>>(requests::endpoint(&c.kind), &[])
                .await?
                .into_iter()
                .map(Title::tidy)
                .collect::<Vec<_>>();
            let mut episodes = BTreeMap::new();
            let mut signatures = HashMap::new();
            if c.kind == "sonarr" {
                for series in &titles {
                    let signature = series.signature();
                    let known = current
                        .and_then(|inventory| inventory.episodes.get(&series.id))
                        .filter(|_| tracker.signatures.get(&series.id) == Some(&signature));
                    let list = match known {
                        Some(list) => list.clone(),
                        None => match c
                            .fetch::<Vec<Episode>>(
                                "episode",
                                &[("seriesId", series.id.to_string())],
                            )
                            .await
                        {
                            Ok(list) => list.into_iter().map(Episode::tidy).collect(),
                            // One failing series keeps its last episodes; without a
                            // signature it is read again with the next title list.
                            Err(error) => {
                                tracing::debug!(series = series.id, error = %error.2, "Keeping the last episode list");
                                episodes.insert(
                                    series.id,
                                    current
                                        .and_then(|inventory| inventory.episodes.get(&series.id))
                                        .cloned()
                                        .unwrap_or_default(),
                                );
                                continue;
                            }
                        },
                    };
                    episodes.insert(series.id, list);
                    signatures.insert(series.id, signature);
                }
            }
            tracker.signatures = signatures;
            tracker.refreshed = Some(std::time::Instant::now());
            Inventory::new(&c.grant.service, titles, episodes, queue)
        }
    };
    tracker.queue = keys;
    Ok(next)
}

async fn queue(c: &Connection<'_>) -> Result<Vec<QueueItem>> {
    let unknown = if c.kind == "radarr" {
        "includeUnknownMovieItems"
    } else {
        "includeUnknownSeriesItems"
    };
    let mut records = Vec::new();
    for page in 1..=QUEUE_PAGES {
        let batch = c
            .fetch::<QueuePage>(
                "queue",
                &[
                    ("page", page.to_string()),
                    ("pageSize", QUEUE_PAGE.to_string()),
                    (unknown, "false".into()),
                ],
            )
            .await?;
        let count = batch.records.len();
        records.extend(batch.records);
        if count < QUEUE_PAGE || records.len() as i64 >= batch.total_records {
            break;
        }
    }
    records.sort_by_key(|r| r.id);
    Ok(records)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Phase {
    Available,
    Partial,
    Downloading,
    Queued,
    Upcoming,
    Missing,
    Unmonitored,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Counts {
    available: usize,
    total: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Status {
    state: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    eta: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    episodes: Option<Counts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    downloading: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    monitored: Option<bool>,
}
impl Status {
    fn new(state: Phase) -> Self {
        Self {
            state,
            progress: None,
            eta: None,
            release: None,
            episodes: None,
            downloading: None,
            monitored: None,
        }
    }
    /// Users learn whether something is coming, not how the library is administered.
    fn for_viewer(mut self, admin: bool) -> Self {
        if !admin {
            if matches!(self.state, Phase::Missing | Phase::Unmonitored) {
                self.state = Phase::Unavailable;
            }
            self.monitored = None;
        }
        self
    }
}

enum Stage {
    Queued,
    Downloading,
    Importing,
}
fn stage(record: &QueueItem) -> Option<Stage> {
    let status = record
        .status
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tracked = record
        .tracked_download_state
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(status.as_str(), "failed" | "warning") {
        return (tracked == "downloading").then_some(Stage::Downloading);
    }
    // A finished download waits for, or is blocked before, its import.
    if matches!(
        tracked.as_str(),
        "importpending" | "importing" | "importblocked"
    ) || status == "completed"
    {
        return Some(Stage::Importing);
    }
    if status == "downloading" {
        return Some(Stage::Downloading);
    }
    matches!(
        status.as_str(),
        "queued" | "delay" | "paused" | "downloadclientunavailable"
    )
    .then_some(Stage::Queued)
}
/// Byte-weighted progress across queue records; importing counts as complete.
fn transfer<'a>(records: impl IntoIterator<Item = &'a QueueItem>) -> Option<Status> {
    let (mut found, mut active, mut done, mut size, mut eta) = (false, false, 0i64, 0i64, None);
    for record in records {
        let Some(stage) = stage(record) else {
            continue;
        };
        found = true;
        eta = eta.max(time(record.estimated_completion_time.as_deref()));
        match stage {
            Stage::Queued => {}
            Stage::Importing => {
                active = true;
                done += record.size.max(1);
                size += record.size.max(1);
            }
            Stage::Downloading => {
                active = true;
                if record.size > 0 {
                    done += record.size - record.sizeleft.clamp(0, record.size);
                    size += record.size;
                }
            }
        }
    }
    found.then(|| {
        let mut status = Status::new(if active {
            Phase::Downloading
        } else {
            Phase::Queued
        });
        status.progress = (active && size > 0).then(|| (done as f64 / size as f64).clamp(0.0, 1.0));
        status.eta = eta;
        status
    })
}

fn movie_status(inventory: &Inventory, movie: &Title, library: bool, now: i64) -> Status {
    let releases = movie.releases().collect::<Vec<_>>();
    let released = movie.is_available.unwrap_or_else(|| {
        movie.status.as_deref() == Some("released") || releases.iter().any(|at| *at <= now)
    });
    let mut status = if movie.has_file || library {
        Status::new(Phase::Available)
    } else if let Some(status) = transfer(
        inventory
            .queue
            .iter()
            .filter(|r| r.movie_id == Some(movie.id)),
    ) {
        status
    } else if !movie.monitored {
        // Unmonitored titles will not arrive, released or not.
        Status::new(Phase::Unmonitored)
    } else if !released {
        Status::new(Phase::Upcoming)
    } else {
        Status::new(Phase::Missing)
    };
    status.release = releases.into_iter().filter(|at| *at > now).min();
    status.monitored = Some(movie.monitored);
    status
}

fn episode_status(
    inventory: &Inventory,
    series: &Title,
    episode: &Episode,
    library: bool,
    now: i64,
) -> Status {
    let mut status = if episode.has_file || library {
        Status::new(Phase::Available)
    } else if let Some(status) = transfer(
        inventory
            .queue
            .iter()
            .filter(|r| r.episode_id == Some(episode.id)),
    ) {
        status
    } else if !(series.monitored && episode.monitored) {
        Status::new(Phase::Unmonitored)
    } else if !episode.aired(now) {
        Status::new(Phase::Upcoming)
    } else {
        Status::new(Phase::Missing)
    };
    status.release = episode.air.filter(|at| *at > now);
    status.monitored = Some(episode.monitored);
    status
}

/// A season or whole show from its episodes; `library` holds Sonarr episode IDs that
/// are playable here even if Sonarr has no file for them.
fn episodes_status(
    inventory: &Inventory,
    series: &Title,
    episodes: &[&Episode],
    library: &HashSet<i64>,
    monitored: bool,
    now: i64,
) -> Status {
    let (mut available, mut total, mut wanted) = (0, 0, false);
    let mut queued = HashSet::new();
    for episode in episodes {
        let aired = episode.aired(now);
        let present = episode.has_file || library.contains(&episode.id);
        // An episode obtained before its air date still counts towards the total.
        total += usize::from(aired || present);
        if present {
            available += 1;
        } else if inventory
            .queue
            .iter()
            .any(|r| r.episode_id == Some(episode.id) && stage(r).is_some())
        {
            queued.insert(episode.id);
        } else if aired && series.monitored && episode.monitored {
            wanted = true;
        }
    }
    let mut status = if let Some(mut status) = transfer(
        inventory
            .queue
            .iter()
            .filter(|r| r.episode_id.is_some_and(|id| queued.contains(&id))),
    ) {
        status.downloading = Some(queued.len());
        status
    } else if total > 0 && available >= total {
        Status::new(Phase::Available)
    } else if available > 0 {
        Status::new(Phase::Partial)
    } else if total == 0 && monitored {
        Status::new(Phase::Upcoming)
    } else if wanted {
        Status::new(Phase::Missing)
    } else {
        Status::new(Phase::Unmonitored)
    };
    status.episodes = Some(Counts { available, total });
    status.release = episodes
        .iter()
        .filter_map(|e| e.air)
        .filter(|at| *at > now)
        .min();
    status.monitored = Some(monitored);
    status
}

fn season_episodes<'a>(found: Found<'a>, season: Option<i64>) -> Vec<&'a Episode> {
    found
        .inventory
        .episodes
        .get(&found.title.id)
        .into_iter()
        .flatten()
        .filter(|e| season.map_or(e.season_number != 0, |n| e.season_number == n))
        .collect()
}
fn show_status(found: Found<'_>, library: &HashSet<i64>, now: i64) -> Status {
    episodes_status(
        found.inventory,
        found.title,
        &season_episodes(found, None),
        library,
        found.title.monitored,
        now,
    )
}
fn season_status(found: Found<'_>, season: i64, library: &HashSet<i64>, now: i64) -> Status {
    let monitored = found
        .title
        .seasons
        .iter()
        .find(|s| s.season_number == season)
        .is_some_and(|s| s.monitored);
    episodes_status(
        found.inventory,
        found.title,
        &season_episodes(found, Some(season)),
        library,
        monitored,
        now,
    )
}

/// How one catalog item names its manager title.
#[derive(Clone, Debug, Default)]
pub(super) struct Link {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) available: bool,
    pub(super) service_id: Option<String>,
    pub(super) entity_id: Option<i64>,
    pub(super) external_id: Option<String>,
    pub(super) tmdb_id: Option<i64>,
    pub(super) tvdb_id: Option<i64>,
    /// The season number of a season, or of an episode's season.
    pub(super) season: Option<i64>,
    pub(super) episode: Option<i64>,
    /// Confirmed manager episode identities of an episode.
    pub(super) mapped: Vec<(String, i64)>,
}
/// A playable library episode within a show or season.
#[derive(Clone, Debug, Default)]
pub(super) struct Present {
    pub(super) season: Option<i64>,
    pub(super) episode: Option<i64>,
    pub(super) mapped: Vec<(String, i64)>,
}
#[derive(Default)]
pub(super) struct Catalog {
    pub(super) links: Vec<Link>,
    pub(super) present: HashMap<String, Vec<Present>>,
}
/// Manager titles that already have a catalog movie or show.
#[derive(Default)]
pub(super) struct Linked {
    pub(super) entities: HashSet<(String, i64)>,
    pub(super) tmdb: HashSet<i64>,
    pub(super) tvdb: HashSet<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ref {
    Tmdb(i64),
    Tvdb(i64),
}
impl Ref {
    fn parse(value: &str) -> Option<Self> {
        let (provider, id) = value.split_once(':')?;
        let id = Some(id)
            .filter(|id| (1..=12).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()))?
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)?;
        match provider {
            "tmdb" => Some(Self::Tmdb(id)),
            "tvdb" => Some(Self::Tvdb(id)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Found<'a> {
    inventory: &'a Inventory,
    title: &'a Title,
}
fn manager(kind: &str) -> Option<&'static str> {
    match kind {
        "movie" => Some("radarr"),
        "show" | "season" | "episode" => Some("sonarr"),
        _ => None,
    }
}
/// The bound manager title, else the one with the same provider identity.
fn find<'a>(snapshot: &'a Snapshot, manager: &str, link: &Link) -> Option<Found<'a>> {
    if let (Some(service), Some(entity)) = (&link.service_id, link.entity_id)
        && let Some(inventory) = snapshot.services.get(manager)
        && inventory.service == *service
        && let Some(title) = inventory.titles.get(&entity)
        && link.external_id.as_deref().is_none_or(|external| {
            let current = if manager == "radarr" {
                title.tmdb_id
            } else {
                title.tvdb_id
            };
            current.map(|v| v.to_string()).as_deref() == Some(external)
        })
    {
        return Some(Found { inventory, title });
    }
    let by_tvdb = (manager == "sonarr").then_some(link.tvdb_id).flatten();
    by_tvdb
        .and_then(|id| find_ref(snapshot, manager, Ref::Tvdb(id)))
        .or_else(|| {
            link.tmdb_id
                .and_then(|id| find_ref(snapshot, manager, Ref::Tmdb(id)))
        })
}
fn find_ref<'a>(snapshot: &'a Snapshot, manager: &str, reference: Ref) -> Option<Found<'a>> {
    let inventory = snapshot.services.get(manager)?;
    let id = match reference {
        Ref::Tmdb(id) => inventory.by_tmdb.get(&id),
        Ref::Tvdb(id) => inventory.by_tvdb.get(&id),
    }?;
    Some(Found {
        inventory,
        title: inventory.titles.get(id)?,
    })
}
/// A confirmed mapping in this series first, else the same season and episode numbers.
fn episode_for<'a>(
    found: Found<'a>,
    mapped: &[(String, i64)],
    season: Option<i64>,
    number: Option<i64>,
) -> Option<&'a Episode> {
    let list = found.inventory.episodes.get(&found.title.id)?;
    mapped
        .iter()
        .filter(|(service, _)| *service == found.inventory.service)
        .find_map(|(_, id)| list.iter().find(|e| e.id == *id))
        .or_else(|| {
            let (season, number) = (season?, number?);
            list.iter()
                .find(|e| e.season_number == season && e.episode_number == number)
        })
}
fn present_ids(found: Found<'_>, present: &[Present]) -> HashSet<i64> {
    present
        .iter()
        .filter_map(|p| episode_for(found, &p.mapped, p.season, p.episode))
        .map(|e| e.id)
        .collect()
}
/// The raw status of one catalog item, before it is shaped for its viewer.
fn link_status(snapshot: &Snapshot, link: &Link, present: &[Present], now: i64) -> Option<Status> {
    let found = find(snapshot, manager(&link.kind)?, link)?;
    Some(match link.kind.as_str() {
        "movie" => movie_status(found.inventory, found.title, link.available, now),
        "show" => show_status(found, &present_ids(found, present), now),
        "season" => season_status(found, link.season?, &present_ids(found, present), now),
        _ => episode_status(
            found.inventory,
            found.title,
            episode_for(found, &link.mapped, link.season, link.episode)?,
            link.available,
            now,
        ),
    })
}
fn artwork_url(found: Found<'_>, kind: &str, grant: &str) -> Option<String> {
    found.title.image(kind).map(|_| {
        format!(
            "/api/v1/catalog/titles/{}/{}/artwork/{kind}?grant={grant}",
            found.inventory.service, found.title.id
        )
    })
}
fn admin(p: &Principal) -> bool {
    p.user.role == Role::Admin
}

/// Add each card's status and the landscape backdrop of its movie or series; an item
/// without a manager title gets `status: null` and `backdrop_url: null`.
pub(crate) async fn decorate(
    state: &AppState,
    p: &Principal,
    items: &mut [&mut Value],
    grant: &str,
) -> Result<()> {
    decorated(state, p, items, grant).await.map(drop)
}

/// [`decorate`] movies or shows, returning what library lists sort their manager titles
/// by.
pub(crate) async fn decorate_titles(
    state: &AppState,
    p: &Principal,
    items: &mut [&mut Value],
    grant: &str,
) -> Result<HashMap<String, Facts>> {
    let (snapshot, catalog) = decorated(state, p, items, grant).await?;
    Ok(catalog
        .links
        .iter()
        .filter_map(|link| {
            let found = find(&snapshot, manager(&link.kind)?, link)?;
            Some((link.id.clone(), found.title.facts()))
        })
        .collect())
}

async fn decorated(
    state: &AppState,
    p: &Principal,
    items: &mut [&mut Value],
    grant: &str,
) -> Result<(Arc<Snapshot>, Catalog)> {
    let snapshot = snapshot(state);
    let catalog = if snapshot.services.is_empty() {
        Catalog::default()
    } else {
        let ids = items
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_owned))
            .collect();
        storage::catalog(&state.db, ids).await?
    };
    let now = now();
    let statuses = catalog
        .links
        .iter()
        .filter_map(|link| {
            let present = catalog.present.get(&link.id).map_or(&[][..], Vec::as_slice);
            link_status(&snapshot, link, present, now)
                .map(|status| (link.id.as_str(), status.for_viewer(admin(p))))
        })
        .collect::<HashMap<_, _>>();
    let backdrops = catalog
        .links
        .iter()
        .filter_map(|link| {
            find(&snapshot, manager(&link.kind)?, link)
                .and_then(|found| artwork_url(found, "fanart", grant))
                .map(|url| (link.id.as_str(), url))
        })
        .collect::<HashMap<_, _>>();
    for item in items {
        let id = item["id"].as_str().map(str::to_owned);
        item["status"] = id
            .as_deref()
            .and_then(|id| statuses.get(id))
            .map_or(Value::Null, |status| json!(status));
        item["backdrop_url"] = json!(id.as_deref().and_then(|id| backdrops.get(id)));
    }
    Ok((snapshot, catalog))
}

/// A detail page's status and its manager backdrop, through the item's movie or show.
pub(crate) async fn detail(
    state: &AppState,
    p: &Principal,
    item: &mut Value,
    grant: &str,
) -> Result<()> {
    let snapshot = snapshot(state);
    let id = item["id"].as_str().unwrap_or_default().to_owned();
    let catalog = if snapshot.services.is_empty() {
        Catalog::default()
    } else {
        storage::catalog(&state.db, vec![id]).await?
    };
    let link = catalog.links.first();
    let present = link
        .and_then(|link| catalog.present.get(&link.id))
        .map_or(&[][..], Vec::as_slice);
    item["status"] = link
        .and_then(|link| link_status(&snapshot, link, present, now()))
        .map_or(Value::Null, |status| json!(status.for_viewer(admin(p))));
    item["backdrop_url"] = json!(
        link.and_then(|link| find(&snapshot, manager(&link.kind)?, link))
            .and_then(|found| artwork_url(found, "fanart", grant))
    );
    Ok(())
}

/// The landscape backdrop and the logo a movie or series has in its manager.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TitleArt {
    pub(crate) backdrop: bool,
    pub(crate) logo: bool,
}

/// The manager images of each catalog item's movie or series; seasons and episodes
/// get their series'. Items without a manager title are left out.
pub(crate) async fn title_art(
    state: &AppState,
    ids: Vec<String>,
) -> Result<HashMap<String, TitleArt>> {
    let snapshot = snapshot(state);
    if snapshot.services.is_empty() || ids.is_empty() {
        return Ok(HashMap::new());
    }
    let links = storage::links(&state.db, ids).await?;
    Ok(links
        .iter()
        .filter_map(|link| {
            let found = find(&snapshot, manager(&link.kind)?, link)?;
            let art = TitleArt {
                backdrop: found.title.image("fanart").is_some(),
                logo: found.title.image("clearlogo").is_some(),
            };
            (art != TitleArt::default()).then(|| (link.id.clone(), art))
        })
        .collect())
}

/// Serve the `fanart` or `clearlogo` of a catalog movie's or series' manager title.
pub(crate) async fn serve_title_art(
    state: &AppState,
    media_id: &str,
    kind: &str,
    request: Request<Body>,
) -> Result<Response> {
    let links = storage::links(&state.db, vec![media_id.to_owned()]).await?;
    let title = {
        let snapshot = snapshot(state);
        links
            .first()
            .and_then(|link| find(&snapshot, manager(&link.kind)?, link))
            .map(|found| (found.inventory.service.clone(), found.title.id))
    };
    let (service, entity) = title.ok_or_else(ApiError::not_found)?;
    serve_title(state, &service, entity, kind, request, None).await
}

/// What library lists sort and filter a movie or show by, beyond its card.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Facts {
    pub(crate) added: Option<i64>,
    pub(crate) rating: Option<f64>,
    pub(crate) genres: Vec<String>,
    /// Bytes of its files.
    pub(crate) size: Option<i64>,
    /// Minutes.
    pub(crate) runtime: Option<i64>,
    /// Its first release day.
    pub(crate) released: Option<i64>,
    /// Episodes with a file.
    pub(crate) episodes: Option<i64>,
    pub(crate) last_aired: Option<i64>,
    pub(crate) next_airing: Option<i64>,
}

/// Every manager movie or series without a catalog item, wanted or not, so the library
/// lists everything Radarr and Sonarr know.
pub(crate) async fn virtual_titles(
    state: &AppState,
    p: &Principal,
    kind: &str,
    grant: &str,
) -> Result<Vec<(Value, Facts)>> {
    let snapshot = snapshot(state);
    let Some(manager) = manager(kind).filter(|_| matches!(kind, "movie" | "show")) else {
        return Ok(Vec::new());
    };
    if !snapshot.services.contains_key(manager) {
        return Ok(Vec::new());
    }
    let linked = storage::linked(&state.db, kind.to_owned()).await?;
    Ok(virtual_items(
        &snapshot,
        kind,
        &linked,
        admin(p),
        now(),
        grant,
    ))
}
fn virtual_items(
    snapshot: &Snapshot,
    kind: &str,
    linked: &Linked,
    admin: bool,
    now: i64,
    grant: &str,
) -> Vec<(Value, Facts)> {
    let Some(manager) = manager(kind) else {
        return Vec::new();
    };
    let Some(inventory) = snapshot.services.get(manager) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for title in inventory.titles.values() {
        let known = linked
            .entities
            .contains(&(inventory.service.clone(), title.id))
            || title.tmdb_id.is_some_and(|id| linked.tmdb.contains(&id))
            || (manager == "sonarr" && title.tvdb_id.is_some_and(|id| linked.tvdb.contains(&id)));
        if known {
            continue;
        }
        let Some(reference) = title.reference(manager) else {
            continue;
        };
        let found = Found { inventory, title };
        let status = if manager == "radarr" {
            movie_status(inventory, title, false, now)
        } else {
            show_status(found, &HashSet::new(), now)
        };
        let mut card = json!({"id":reference,"kind":kind,"title":title.title,"year":title.year,"available":false,"virtual":true,
            "artwork_url":artwork_url(found,"poster",grant),"status":status.for_viewer(admin),"watched":false});
        if manager == "sonarr" {
            card["ended"] = json!(ended(title.status.as_deref()));
        }
        items.push((card, title.facts()));
    }
    items
}

/// The TMDB and TVDB ids of every movie or show the library lists: its own items and
/// the Radarr or Sonarr titles shown beside them.
pub(super) async fn library_ids(state: &AppState, kind: &str) -> Result<Linked> {
    let mut linked = storage::linked(&state.db, kind.to_owned()).await?;
    if let Some(inventory) = manager(kind).and_then(|m| snapshot(state).services.get(m).cloned()) {
        linked.tmdb.extend(inventory.by_tmdb.keys());
        if kind == "show" {
            linked.tvdb.extend(inventory.by_tvdb.keys());
        }
    }
    Ok(linked)
}

/// A library season and its episodes, with the caller's own progress.
#[derive(Clone, Debug, Default)]
pub(super) struct LibrarySeason {
    pub(super) id: String,
    pub(super) number: Option<i64>,
    pub(super) title: String,
    pub(super) episodes: Vec<LibraryEpisode>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct LibraryEpisode {
    pub(super) id: String,
    pub(super) number: Option<i64>,
    pub(super) title: String,
    pub(super) overview: Option<String>,
    pub(super) air: Option<i64>,
    pub(super) runtime: Option<i64>,
    pub(super) available: bool,
    pub(super) watched: bool,
    pub(super) position: Option<f64>,
    pub(super) duration: Option<f64>,
    pub(super) mapped: Vec<(String, i64)>,
}

#[derive(Default)]
struct Group<'a> {
    id: Option<&'a str>,
    title: Option<&'a str>,
    watched: Vec<bool>,
    episodes: Vec<(i64, Value)>,
}
/// Library seasons merged with Sonarr's episode list; Specials come last.
fn seasons_value(
    found: Option<Found<'_>>,
    library: &[LibrarySeason],
    admin: bool,
    now: i64,
) -> Value {
    let mut groups = BTreeMap::<i64, Group<'_>>::new();
    let mut linked = HashSet::new();
    let mut present = HashSet::new();
    for season in library {
        let number = season.number.unwrap_or(0);
        let group = groups.entry(number).or_default();
        group.id = group.id.or(Some(season.id.as_str()));
        group.title = group.title.or(Some(season.title.as_str()));
        for episode in &season.episodes {
            let upstream = found.and_then(|found| {
                episode_for(found, &episode.mapped, Some(number), episode.number)
            });
            if let Some(upstream) = upstream {
                linked.insert(upstream.id);
                if episode.available {
                    present.insert(upstream.id);
                }
            }
            let status = found.zip(upstream).map(|(found, upstream)| {
                episode_status(
                    found.inventory,
                    found.title,
                    upstream,
                    episode.available,
                    now,
                )
                .for_viewer(admin)
            });
            let episode_number = episode
                .number
                .or(upstream.map(|e| e.episode_number))
                .unwrap_or(0);
            group.watched.push(episode.watched);
            group.episodes.push((episode_number, json!({
                "id":episode.id,"season":number,"number":episode_number,
                "title":upstream.and_then(|e| e.title.as_deref()).unwrap_or(episode.title.as_str()),
                "overview":upstream.and_then(|e| e.overview.as_deref()).or(episode.overview.as_deref()),
                "air_date":upstream.and_then(|e| e.air).or(episode.air),
                "runtime":upstream.and_then(|e| e.runtime).or(episode.runtime),
                "available":episode.available,"watched":episode.watched,
                "position":episode.position,"duration":episode.duration,"status":status
            })));
        }
    }
    if let Some(found) = found {
        for episode in found
            .inventory
            .episodes
            .get(&found.title.id)
            .into_iter()
            .flatten()
            .filter(|e| !linked.contains(&e.id))
        {
            let status =
                episode_status(found.inventory, found.title, episode, false, now).for_viewer(admin);
            groups
                .entry(episode.season_number)
                .or_default()
                .episodes
                .push((
                    episode.episode_number,
                    json!({
                        "id":null,"season":episode.season_number,"number":episode.episode_number,
                        "title":episode.title.clone().filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| format!("Episode {}", episode.episode_number)),
                        "overview":episode.overview,"air_date":episode.air,
                        "runtime":episode.runtime,"available":false,"watched":false,
                        "position":null,"duration":null,"status":status
                    }),
                ));
        }
    }
    let mut groups = groups
        .into_iter()
        .filter(|(_, group)| !group.episodes.is_empty())
        .collect::<Vec<_>>();
    groups.sort_by_key(|(number, _)| (*number == 0, *number));
    let seasons = groups
        .into_iter()
        .map(|(number, mut group)| {
            group.episodes.sort_by_key(|(episode, _)| *episode);
            let status = found
                .filter(|found| !season_episodes(*found, Some(number)).is_empty())
                .map(|found| season_status(found, number, &present, now).for_viewer(admin));
            let title = group.title.map_or_else(
                || {
                    if number == 0 {
                        "Specials".to_owned()
                    } else {
                        format!("Season {number}")
                    }
                },
                str::to_owned,
            );
            json!({"number":number,"id":group.id,"title":title,"status":status,
                "watched":!group.watched.is_empty() && group.watched.iter().all(|w| *w),
                "episodes":group.episodes.into_iter().map(|(_, episode)| episode).collect::<Vec<_>>()})
        })
        .collect::<Vec<_>>();
    json!({"seasons":seasons})
}

async fn seasons(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::Browse).await?;
    let snapshot = snapshot(&state);
    let reference = Ref::parse(&id);
    let show = match reference {
        Some(reference) => storage::lookup(&state.db, "show".into(), reference).await?,
        None => Some(id),
    };
    let link = match &show {
        Some(show) => storage::catalog(&state.db, vec![show.clone()])
            .await?
            .links
            .into_iter()
            .find(|link| link.kind == "show"),
        None => None,
    };
    let found = link
        .as_ref()
        .and_then(|link| find(&snapshot, "sonarr", link))
        .or_else(|| reference.and_then(|r| find_ref(&snapshot, "sonarr", r)));
    if link.is_none() && found.is_none() {
        return Err(ApiError::not_found());
    }
    let library = match &link {
        Some(link) => storage::show(&state.db, link.id.clone(), p.user.id.clone()).await?,
        None => Vec::new(),
    };
    Ok(Json(seasons_value(found, &library, admin(&p), now())))
}

#[derive(Deserialize)]
struct Lookup {
    kind: String,
    #[serde(rename = "ref")]
    reference: String,
}
async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Lookup>,
) -> Result<Json<Value>> {
    let p = security::require(&state, &headers, Capability::Browse).await?;
    let reference = Ref::parse(&query.reference)
        .filter(|r| query.kind == "show" || matches!(r, Ref::Tmdb(_)))
        .filter(|_| matches!(query.kind.as_str(), "movie" | "show"))
        .ok_or_else(|| ApiError::bad("Choose a movie or show and a tmdb: or tvdb: reference"))?;
    let manager = manager(&query.kind).ok_or_else(ApiError::not_found)?;
    let media = storage::lookup(&state.db, query.kind.clone(), reference).await?;
    let snapshot = snapshot(&state);
    let catalog = match &media {
        Some(media) if !snapshot.services.is_empty() => {
            storage::catalog(&state.db, vec![media.clone()]).await?
        }
        _ => Catalog::default(),
    };
    let link = catalog.links.first();
    let found = link
        .and_then(|link| find(&snapshot, manager, link))
        .or_else(|| find_ref(&snapshot, manager, reference));
    let now = now();
    let status = match (link, found) {
        (Some(link), Some(found)) => {
            let present = catalog.present.get(&link.id).map_or(&[][..], Vec::as_slice);
            Some(if manager == "radarr" {
                movie_status(found.inventory, found.title, link.available, now)
            } else {
                show_status(found, &present_ids(found, present), now)
            })
        }
        (None, Some(found)) => Some(if manager == "radarr" {
            movie_status(found.inventory, found.title, false, now)
        } else {
            show_status(found, &HashSet::new(), now)
        }),
        _ => None,
    };
    let title = match found {
        Some(found) => {
            let grant = crate::grants::artwork(&state, &p).await?;
            let t = found.title;
            json!({"title":t.title,"year":t.year,"overview":t.overview,"runtime":t.runtime,"genres":t.genres,
                "tmdb_id":t.tmdb_id,"tvdb_id":if manager == "sonarr" {t.tvdb_id} else {None},"imdb_id":t.imdb_id,
                "ended":manager == "sonarr" && ended(t.status.as_deref()),
                "poster_url":artwork_url(found,"poster",&grant),"backdrop_url":artwork_url(found,"fanart",&grant)})
        }
        None => Value::Null,
    };
    Ok(Json(json!({"media_id":media,"title":title,
        "status":status.map(|status| status.for_viewer(admin(&p)))})))
}

#[derive(Deserialize)]
struct ArtGrant {
    grant: String,
    width: Option<u32>,
}
/// Manager artwork for a title, cached on disk and fetched only from the manager itself.
async fn artwork(
    State(state): State<AppState>,
    Path((service_id, entity, kind)): Path<(String, String, String)>,
    Query(query): Query<ArtGrant>,
    request: Request<Body>,
) -> Result<Response> {
    crate::grants::resolve(&state, &query.grant, "artwork", false)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    crate::artwork::validate_width(query.width)?;
    let entity = entity
        .parse::<i64>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(ApiError::not_found)?;
    if !matches!(kind.as_str(), "poster" | "fanart")
        || !(1..=64).contains(&service_id.len())
        || !service_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(ApiError::not_found());
    }
    serve_title(&state, &service_id, entity, &kind, request, query.width).await
}
/// One manager image of a title from the cache, downloaded from the manager when it changed.
async fn serve_title(
    state: &AppState,
    service_id: &str,
    entity: i64,
    kind: &str,
    request: Request<Body>,
    width: Option<u32>,
) -> Result<Response> {
    let local = snapshot(state)
        .service(service_id)
        .and_then(|inventory| inventory.titles.get(&entity))
        .and_then(|title| title.image(kind))
        .map(str::to_owned)
        .ok_or_else(ApiError::not_found)?;
    let dir = state.config.cache.join("artwork");
    let name = format!("title-{service_id}-{entity}-{kind}");
    let path = dir.join(&name);
    if !cached(&dir, &name, &local).await {
        // A page of new cards asks for many images at once: fetch a few at a time,
        // and only those another request has not just cached.
        static DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
        let _permit = DOWNLOADS.acquire().await.map_err(anyhow::Error::from)?;
        if cached(&dir, &name, &local).await {
            return super::metadata::serve_artwork(path, request, width).await;
        }
        match download(state, service_id, &dir, &name, &local).await {
            Ok(true) => {}
            // A stale image is better than none while the manager is unreachable.
            Ok(false) | Err(_) if path.is_file() => {}
            Ok(false) => return Err(ApiError::not_found()),
            Err(error) => {
                tracing::debug!(error = %error.2, "Manager artwork is unavailable");
                return Err(ApiError::not_found());
            }
        }
    }
    super::metadata::serve_artwork(path, request, width).await
}
/// What decides which cached title artwork is still wanted: each service's titles.
fn artwork_key(snapshot: &Snapshot) -> u64 {
    let mut hasher = DefaultHasher::new();
    for inventory in snapshot.services.values() {
        (&inventory.service, inventory.library).hash(&mut hasher);
    }
    hasher.finish()
}

/// Whether a cached file still belongs to a title. Files that are not title artwork
/// are left alone, and so are services not read since startup: a slow manager must
/// not cost its cache. Artwork of a removed service or title, or an image the title
/// no longer has, is not wanted.
fn artwork_wanted(name: &str, snapshot: &Snapshot, configured: &HashSet<String>) -> bool {
    let Some(rest) = name.strip_prefix("title-") else {
        return true;
    };
    let rest = rest.strip_suffix(".source").unwrap_or(rest);
    let mut parts = rest.rsplitn(3, '-');
    let (Some(kind), Some(entity), Some(service)) = (parts.next(), parts.next(), parts.next())
    else {
        return true;
    };
    let Ok(entity) = entity.parse::<i64>() else {
        return true;
    };
    if !configured.contains(service) {
        return false;
    }
    let Some(inventory) = snapshot.service(service) else {
        return true;
    };
    inventory
        .titles
        .get(&entity)
        .is_some_and(|title| title.image(kind).is_some())
}

/// Delete cached title artwork nothing can show any more, and downloads left half
/// written by an interruption.
async fn prune_artwork(state: &AppState, snapshot: &Snapshot) -> anyhow::Result<usize> {
    let configured = storage::configured(&state.db).await?;
    let dir = state.config.cache.join("artwork");
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut removed = 0;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| name.starts_with("title-")) else {
            continue;
        };
        let unwanted = if name.ends_with(".tmp") {
            entry
                .metadata()
                .await?
                .modified()?
                .elapsed()
                .is_ok_and(|age| age >= std::time::Duration::from_secs(3600))
        } else {
            !artwork_wanted(name, snapshot, &configured)
        };
        if unwanted {
            match tokio::fs::remove_file(entry.path()).await {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(removed)
}

/// Whether this image is on disk and was fetched from the manager's current path.
async fn cached(dir: &std::path::Path, name: &str, local: &str) -> bool {
    tokio::fs::read_to_string(dir.join(format!("{name}.source")))
        .await
        .is_ok_and(|source| source == local)
        && dir.join(name).is_file()
}
async fn download(
    state: &AppState,
    service_id: &str,
    dir: &std::path::Path,
    name: &str,
    local: &str,
) -> Result<bool> {
    let s = service(state, service_id).await?;
    let c = Connection::open(state, &s).await?;
    let Some(bytes) = super::metadata::fetch_image(state, &c, local).await? else {
        return Ok(false);
    };
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(anyhow::Error::from)?;
    super::metadata::store_artwork(dir, name, bytes).await?;
    tokio::fs::write(dir.join(format!("{name}.source")), local)
        .await
        .map_err(anyhow::Error::from)?;
    Ok(true)
}
