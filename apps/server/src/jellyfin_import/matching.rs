//! Finds the titles here behind each item of a Jellyfin watch history. Both servers
//! usually read the same files, so a file names its title best; provider IDs from Radarr
//! and Sonarr come next, and titles last, for an administrator to confirm.

use super::client::Item;
use std::collections::{BTreeMap, HashMap};

/// What the import can match against, read from the catalog.
#[derive(Default)]
pub(super) struct Catalog {
    pub files: Vec<File>,
    pub movies: Vec<Title>,
    pub shows: Vec<Title>,
    pub episodes: Vec<Episode>,
}
/// A present file and one title it carries; a multi-episode file has a row per episode.
pub(super) struct File {
    pub path: String,
    pub edition: String,
    pub media: String,
    pub kind: String,
    pub show: Option<String>,
    pub number: Option<i64>,
}
pub(super) struct Title {
    pub id: String,
    /// The title shown here first, then the one read from the file name.
    pub titles: Vec<String>,
    pub year: Option<i64>,
    pub tmdb: Option<String>,
    pub imdb: Option<String>,
    pub tvdb: Option<String>,
}
pub(super) struct Episode {
    pub id: String,
    pub show: String,
    pub season: i64,
    pub number: i64,
    pub tvdb: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    File,
    Ids,
    Title,
}

/// Where one Jellyfin item's state goes.
pub(super) struct Target {
    /// Movies or episodes, or the show of a favorite series.
    pub media: Vec<String>,
    /// The title among them that carries a resume point.
    pub resume: String,
    /// The edition of the file Jellyfin played, when the file matched.
    pub edition: Option<String>,
    /// Set when only a title matched, so an administrator can leave it out.
    pub group: Option<String>,
}

pub(super) enum Outcome {
    Found(Target),
    /// Matched by title in a group the administrator left out.
    Excluded,
    Missing,
}

/// Jellyfin titles that matched here by title alone.
#[derive(Clone)]
pub(super) struct Group {
    pub key: String,
    pub jellyfin: String,
    pub title: String,
    pub series: bool,
    pub items: usize,
}

struct Hit<'a> {
    parts: Vec<&'a str>,
    kind: &'a str,
    edition: &'a str,
    /// Each title in the file, with its show and episode number.
    media: Vec<(&'a str, Option<&'a str>, Option<i64>)>,
}

type Candidates<'a> = HashMap<String, Vec<(&'a str, Option<i64>)>>;
/// Titles by provider, then by that provider's ID.
type Ids<'a> = HashMap<&'static str, HashMap<&'a str, Vec<&'a str>>>;

pub(super) struct Matcher<'a> {
    files: HashMap<&'a str, Vec<Hit<'a>>>,
    episodes: HashMap<(&'a str, i64, i64), Vec<&'a str>>,
    episode_tvdb: HashMap<&'a str, Vec<&'a str>>,
    movies: HashMap<&'a str, &'a Title>,
    shows: HashMap<&'a str, &'a Title>,
    movie_titles: Candidates<'a>,
    show_titles: Candidates<'a>,
    movie_ids: Ids<'a>,
    show_ids: Ids<'a>,
}

fn parts(path: &str) -> Vec<&str> {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect()
}

/// Lowercase words without punctuation, so "Spider-Man: Homecoming" is "spider man homecoming".
fn words(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.to_lowercase().replace('&', " and ").chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !matches!(c, '\'' | '’') {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words without a trailing year, and that year: "Doctor Who (2005)" is Doctor Who from 2005.
fn split_year(value: &str) -> (String, Option<i64>) {
    let normalized = words(value);
    if let Some((base, last)) = normalized.rsplit_once(' ')
        && last.len() == 4
        && let Ok(year) = last.parse::<i64>()
        && (1900..2100).contains(&year)
    {
        return (base.to_owned(), Some(year));
    }
    (normalized, None)
}

/// The single entry left, or nothing when none or several remain.
fn unique<'a>(ids: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut ids: Vec<&str> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    (ids.len() == 1).then(|| ids[0])
}

fn with_year(title: &str, year: Option<i64>) -> String {
    match year {
        Some(year) => format!("{title} ({year})"),
        None => title.to_owned(),
    }
}

fn index_ids(titles: &[Title]) -> Ids<'_> {
    let mut ids: Ids = HashMap::new();
    for title in titles {
        for (provider, value) in [
            ("Tmdb", &title.tmdb),
            ("Imdb", &title.imdb),
            ("Tvdb", &title.tvdb),
        ] {
            if let Some(value) = value {
                ids.entry(provider)
                    .or_default()
                    .entry(value.as_str())
                    .or_default()
                    .push(&title.id);
            }
        }
    }
    ids
}

impl<'a> Matcher<'a> {
    pub fn new(catalog: &'a Catalog) -> Self {
        let mut by_path: BTreeMap<&str, Hit> = BTreeMap::new();
        for file in &catalog.files {
            by_path
                .entry(&file.path)
                .or_insert_with(|| Hit {
                    parts: parts(&file.path),
                    kind: &file.kind,
                    edition: &file.edition,
                    media: Vec::new(),
                })
                .media
                .push((&file.media, file.show.as_deref(), file.number));
        }
        let mut files: HashMap<&str, Vec<Hit>> = HashMap::new();
        for hit in by_path.into_values() {
            if let Some(name) = hit.parts.last() {
                files.entry(name).or_default().push(hit);
            }
        }
        let mut episodes: HashMap<(&str, i64, i64), Vec<&str>> = HashMap::new();
        let mut episode_tvdb: HashMap<&str, Vec<&str>> = HashMap::new();
        for episode in &catalog.episodes {
            episodes
                .entry((&episode.show, episode.season, episode.number))
                .or_default()
                .push(&episode.id);
            if let Some(tvdb) = &episode.tvdb {
                episode_tvdb.entry(tvdb).or_default().push(&episode.id);
            }
        }
        let mut movie_titles: Candidates = HashMap::new();
        for movie in &catalog.movies {
            for title in &movie.titles {
                movie_titles
                    .entry(words(title))
                    .or_default()
                    .push((&movie.id, movie.year));
            }
        }
        let mut show_titles: Candidates = HashMap::new();
        for show in &catalog.shows {
            for title in &show.titles {
                let (base, year) = split_year(title);
                show_titles
                    .entry(base)
                    .or_default()
                    .push((&show.id, year.or(show.year)));
            }
        }
        Self {
            files,
            episodes,
            episode_tvdb,
            movies: catalog.movies.iter().map(|t| (t.id.as_str(), t)).collect(),
            shows: catalog.shows.iter().map(|t| (t.id.as_str(), t)).collect(),
            movie_titles,
            show_titles,
            movie_ids: index_ids(&catalog.movies),
            show_ids: index_ids(&catalog.shows),
        }
    }

    /// The present file Jellyfin played: the same name with the longest shared path, which
    /// tells apart files mounted at different places.
    fn file(&self, path: Option<&str>, kind: &str) -> Option<&Hit<'a>> {
        let wanted = parts(path?);
        let mut best: Option<(&Hit, usize)> = None;
        let mut tied = false;
        for hit in self
            .files
            .get(*wanted.last()?)?
            .iter()
            .filter(|h| h.kind == kind)
        {
            let shared = wanted
                .iter()
                .rev()
                .zip(hit.parts.iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            match best {
                Some((_, length)) if length > shared => {}
                Some((_, length)) if length == shared => tied = true,
                _ => {
                    best = Some((hit, shared));
                    tied = false;
                }
            }
        }
        best.filter(|_| !tied).map(|(hit, _)| hit)
    }

    fn by_ids(ids: &Ids<'a>, item: &Item, providers: &[&'static str]) -> Option<&'a str> {
        providers.iter().find_map(|provider| {
            let wanted = item.provider(provider)?;
            unique(ids.get(provider)?.get(wanted.as_str())?.iter().copied())
        })
    }

    fn by_title(titles: &Candidates<'a>, name: &str, year: Option<i64>) -> Option<&'a str> {
        unique(
            titles
                .get(name)?
                .iter()
                .filter(|(_, known)| known.is_none() || year.is_none() || *known == year)
                .map(|(id, _)| *id),
        )
    }

    /// Resolves every account's items at once: a series is the show its played files belong
    /// to, so files matched for one person place the other people's episodes too.
    pub fn resolve(
        &self,
        accounts: &[Vec<Item>],
        series: &[Item],
        excluded: &[String],
    ) -> (Vec<Vec<Outcome>>, Vec<Group>) {
        let mut votes: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
        for item in accounts.iter().flatten().filter(|i| i.kind == "Episode") {
            if let (Some(series), Some(hit)) = (
                item.series_id.as_deref(),
                self.file(item.path.as_deref(), "episode"),
            ) {
                for &(_, show, _) in &hit.media {
                    if let Some(show) = show {
                        *votes.entry(series).or_default().entry(show).or_default() += 1;
                    }
                }
            }
        }
        // A series is named by its own item when Jellyfin returned it, else by its episodes.
        let series_items: HashMap<&str, &Item> =
            series.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut names: HashMap<&str, (&str, Option<i64>)> = HashMap::new();
        for item in accounts.iter().flatten() {
            if let (Some(id), Some(name)) = (item.series_id.as_deref(), item.series_name.as_deref())
            {
                names.entry(id).or_insert((name, None));
            }
            if item.kind == "Series" {
                names.insert(item.id.as_str(), (item.name.as_str(), item.production_year));
            }
        }
        for (&id, item) in &series_items {
            names.insert(id, (item.name.as_str(), item.production_year));
        }
        let mut shows: HashMap<&str, (&str, Method)> = HashMap::new();
        for (&series, &(name, year)) in &names {
            let by_votes = votes.get(series).and_then(|counts| {
                let best = counts.values().max()?;
                unique(
                    counts
                        .iter()
                        .filter(|(_, count)| *count == best)
                        .map(|(show, _)| *show),
                )
            });
            let resolved = by_votes
                .map(|show| (show, Method::File))
                .or_else(|| {
                    let item = series_items.get(series)?;
                    Self::by_ids(&self.show_ids, item, &["Tvdb", "Tmdb", "Imdb"])
                        .map(|show| (show, Method::Ids))
                })
                .or_else(|| {
                    let (base, named_year) = split_year(name);
                    Self::by_title(&self.show_titles, &base, year.or(named_year))
                        .map(|show| (show, Method::Title))
                });
            if let Some(resolved) = resolved {
                shows.insert(series, resolved);
            }
        }

        let mut groups: BTreeMap<String, (Group, Vec<&str>)> = BTreeMap::new();
        let mut resolved = Vec::with_capacity(accounts.len());
        for items in accounts {
            let mut outcomes = Vec::with_capacity(items.len());
            for item in items {
                let outcome = match self.target(item, &shows) {
                    None => Outcome::Missing,
                    Some((target, method, _)) if method != Method::Title => Outcome::Found(target),
                    Some((_, _, key)) if excluded.contains(&key) => Outcome::Excluded,
                    Some((target, _, key)) => match self.label(item, &target, &names, &shows) {
                        None => Outcome::Missing,
                        Some((jellyfin, title, series)) => {
                            let (group, seen) = groups.entry(key.clone()).or_insert_with(|| {
                                (
                                    Group {
                                        key: key.clone(),
                                        jellyfin,
                                        title,
                                        series,
                                        items: 0,
                                    },
                                    Vec::new(),
                                )
                            });
                            if !seen.contains(&item.id.as_str()) {
                                seen.push(&item.id);
                                group.items += 1;
                            }
                            Outcome::Found(Target {
                                group: Some(key),
                                ..target
                            })
                        }
                    },
                };
                outcomes.push(outcome);
            }
            resolved.push(outcomes);
        }
        (
            resolved,
            groups.into_values().map(|(group, _)| group).collect(),
        )
    }

    /// How a title match reads to the administrator: the Jellyfin title, the one here, and
    /// whether it stands for a series.
    fn label(
        &self,
        item: &Item,
        target: &Target,
        names: &HashMap<&str, (&str, Option<i64>)>,
        shows: &HashMap<&str, (&str, Method)>,
    ) -> Option<(String, String, bool)> {
        if item.kind == "Movie" {
            let movie = self.movies.get(target.media[0].as_str())?;
            return Some((
                with_year(&item.name, item.production_year),
                with_year(movie.titles.first()?, movie.year),
                false,
            ));
        }
        let series = item.series_id.as_deref().unwrap_or(item.id.as_str());
        let (name, year) = names
            .get(series)
            .copied()
            .unwrap_or((item.name.as_str(), None));
        let &(show, _) = shows.get(series)?;
        let title = self.shows.get(show)?.titles.first()?;
        Some((with_year(name, year), title.clone(), true))
    }

    /// One item's titles here, how they matched, and the key of its title group.
    fn target(
        &self,
        item: &Item,
        shows: &HashMap<&str, (&'a str, Method)>,
    ) -> Option<(Target, Method, String)> {
        let found = |media: Vec<&str>, resume: &str, edition: Option<&str>, method, key| {
            Some((
                Target {
                    media: media.into_iter().map(str::to_owned).collect(),
                    resume: resume.to_owned(),
                    edition: edition.map(str::to_owned),
                    group: None,
                },
                method,
                key,
            ))
        };
        match item.kind.as_str() {
            "Movie" => {
                let key = format!("movie:{}", item.id);
                if let Some(hit) = self.file(item.path.as_deref(), "movie") {
                    let media: Vec<&str> = hit.media.iter().map(|m| m.0).collect();
                    return found(
                        media.clone(),
                        media[0],
                        Some(hit.edition),
                        Method::File,
                        key,
                    );
                }
                let (movie, method) = Self::by_ids(&self.movie_ids, item, &["Tmdb", "Imdb"])
                    .map(|movie| (movie, Method::Ids))
                    .or_else(|| {
                        Self::by_title(&self.movie_titles, &words(&item.name), item.production_year)
                            .map(|movie| (movie, Method::Title))
                    })?;
                found(vec![movie], movie, None, method, key)
            }
            "Episode" => {
                let series = item.series_id.as_deref();
                let key = format!("series:{}", series.unwrap_or(&item.id));
                if let Some(hit) = self.file(item.path.as_deref(), "episode") {
                    let media: Vec<&str> = hit.media.iter().map(|m| m.0).collect();
                    // A file holding several episodes resumes in the one Jellyfin lists first.
                    let resume = hit
                        .media
                        .iter()
                        .find(|m| m.2.is_some() && m.2 == item.index_number)
                        .map_or(media[0], |m| m.0);
                    return found(media.clone(), resume, Some(hit.edition), Method::File, key);
                }
                let by_numbers = |show: &'a str| -> Option<Vec<&'a str>> {
                    let season = item.parent_index_number?;
                    let first = item.index_number?;
                    let last = item
                        .index_number_end
                        .unwrap_or(first)
                        .clamp(first, first + 20);
                    let media: Vec<&str> = (first..=last)
                        .flat_map(|number| {
                            self.episodes
                                .get(&(show, season, number))
                                .into_iter()
                                .flatten()
                                .copied()
                        })
                        .collect();
                    (!media.is_empty()).then_some(media)
                };
                let show = series.and_then(|s| shows.get(s)).copied();
                let (media, method) = show
                    .filter(|&(_, method)| method != Method::Title)
                    .and_then(|(show, method)| Some((by_numbers(show)?, method)))
                    .or_else(|| {
                        let tvdb = item.provider("Tvdb")?;
                        let episode =
                            unique(self.episode_tvdb.get(tvdb.as_str())?.iter().copied())?;
                        Some((vec![episode], Method::Ids))
                    })
                    .or_else(|| {
                        show.filter(|&(_, method)| method == Method::Title)
                            .and_then(|(show, method)| Some((by_numbers(show)?, method)))
                    })?;
                found(media.clone(), media[0], None, method, key)
            }
            "Series" => {
                let &(show, method) = shows.get(item.id.as_str())?;
                found(
                    vec![show],
                    show,
                    None,
                    method,
                    format!("series:{}", item.id),
                )
            }
            _ => None,
        }
    }
}
