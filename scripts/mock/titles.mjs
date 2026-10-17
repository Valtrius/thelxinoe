import { art, now, viewer } from './data.mjs';

const day = 86400;

// Radarr and Sonarr titles without library files, and the download or air
// state of a few library titles. Times are offsets from the current request.
export function titleSeed() {
  return {
    movies: [
      {
        tmdb: 35,
        title: 'The Quiet Harbor',
        year: 2026,
        state: 'downloading',
        progress: 0.42,
        eta: 18 * 60,
      },
      {
        tmdb: 36,
        title: 'Distant Shores',
        year: 2026,
        state: 'upcoming',
        release: 21 * day,
      },
      { tmdb: 37, title: 'Signal Fire', year: 2025, state: 'missing' },
      { tmdb: 38, title: 'Paper Lanterns', year: 2024, state: 'queued' },
    ],
    series: [
      {
        tmdb: 46,
        tvdb: 9046,
        title: 'Ember Coast',
        year: 2026,
        premiere: 10 * day,
        episodes: 6,
      },
    ],
    // Sonarr episodes that have no library file yet.
    episodes: {
      'show-0': [
        { season: 2, number: 5, aired: -3 * day, state: 'missing' },
        { season: 2, number: 6, aired: 4 * day, state: 'upcoming' },
      ],
      'show-1': [
        {
          season: 2,
          number: 5,
          aired: -1 * day,
          state: 'downloading',
          progress: 0.6,
          eta: 25 * 60,
        },
      ],
    },
  };
}

export const titleDiscover = (seed) => [
  ...seed.movies.map((movie, i) => ({
    id: movie.tmdb,
    kind: 'movie',
    title: movie.title,
    date: `${movie.year}-05-01`,
    art: 30 + i,
  })),
  ...seed.series.map((series, i) => ({
    id: series.tmdb,
    kind: 'tv',
    title: series.title,
    date: `${series.year}-11-01`,
    art: 40 + i,
  })),
];

const episodeTitles = [
  'First Contact',
  'The Crossing',
  'The Signal',
  'Homeward',
];

function collapse(status, admin) {
  if (!status || admin) return status;
  const rest = { ...status };
  delete rest.monitored;
  return ['missing', 'unmonitored'].includes(rest.state)
    ? { ...rest, state: 'unavailable' }
    : rest;
}

function download(entry) {
  return {
    state: entry.state,
    ...(entry.progress !== undefined ? { progress: entry.progress } : {}),
    ...(entry.eta ? { eta: now() + entry.eta } : {}),
  };
}

function aggregate(episodes, monitored = true) {
  const aired = episodes.filter((e) => e.air_date && e.air_date <= now());
  const available = episodes.filter((e) => e.available).length;
  const queue = episodes.filter((e) =>
    ['downloading', 'queued'].includes(e.raw?.state),
  );
  const upcoming = episodes
    .map((e) => e.air_date)
    .filter((date) => date && date > now())
    .sort((a, b) => a - b)[0];
  const base = {
    episodes: { available, total: aired.length },
    monitored,
    ...(upcoming ? { release: upcoming } : {}),
  };
  if (queue.length) {
    const active = queue.filter((e) => e.raw.state === 'downloading');
    return {
      ...base,
      state: active.length ? 'downloading' : 'queued',
      downloading: queue.length,
      ...(active.length
        ? {
            progress:
              active.reduce((sum, e) => sum + e.raw.progress, 0) /
              active.length,
          }
        : {}),
      ...(active[0]?.raw.eta ? { eta: now() + active[0].raw.eta } : {}),
    };
  }
  if (aired.length && available >= aired.length)
    return { ...base, state: 'available' };
  if (available) return { ...base, state: 'partial' };
  if (!aired.length) return { ...base, state: 'upcoming' };
  return { ...base, state: monitored ? 'missing' : 'unmonitored' };
}

/** Seasons of a library show or of a Sonarr-only series, raw states. */
function seasonsOf(s, show) {
  const series =
    typeof show === 'string'
      ? null
      : s.titles.series.find((entry) => entry === show);
  if (series) {
    const episodes = Array.from({ length: series.episodes }, (_, i) => ({
      id: null,
      season: 1,
      number: i + 1,
      title: `Chapter ${i + 1}`,
      overview: null,
      air_date: now() + series.premiere + i * 7 * day,
      runtime: 45,
      available: false,
      watched: false,
      position: null,
      duration: null,
      raw: { state: 'upcoming' },
    }));
    return [
      { number: 1, id: null, title: 'Season 1', watched: false, episodes },
    ];
  }
  const index = Number(show.split('-')[1]);
  const extras = s.titles.episodes[show] ?? [];
  return s.library
    .filter((item) => item.kind === 'season' && item.parent_id === show)
    .map((season) => {
      const number = season.metadata.season_number;
      const library = s.library
        .filter(
          (item) => item.kind === 'episode' && item.parent_id === season.id,
        )
        .map((item) => ({
          id: item.id,
          season: number,
          number: item.metadata.episode_number,
          title: item.title,
          overview: item.overview ?? null,
          air_date:
            now() -
            ((2 - number) * 8 + (5 - item.metadata.episode_number)) * 7 * day -
            index * day,
          runtime: Math.round(item.duration / 60),
          available: item.available !== false,
          watched: Boolean(s.mediaStates[item.id]?.watched),
          position: item.position || null,
          duration: item.position ? item.duration : null,
          raw: { state: item.available === false ? 'missing' : 'available' },
        }));
      const sonarr = extras
        .filter((extra) => extra.season === number)
        .map((extra) => ({
          id: null,
          season: number,
          number: extra.number,
          title: episodeTitles[(extra.number - 1) % 4],
          overview: null,
          air_date: now() + extra.aired,
          runtime: 45,
          available: false,
          watched: false,
          position: null,
          duration: null,
          raw: extra,
        }));
      const episodes = [...library, ...sonarr];
      return {
        number,
        id: season.id,
        title: season.title,
        watched:
          library.length > 0 && library.every((episode) => episode.watched),
        episodes,
      };
    });
}

function episodeStatus(episode) {
  if (episode.available) return { state: 'available', monitored: true };
  if (['downloading', 'queued'].includes(episode.raw.state))
    return { ...download(episode.raw), monitored: true };
  if (!episode.air_date || episode.air_date > now())
    return {
      state: 'upcoming',
      monitored: true,
      ...(episode.air_date ? { release: episode.air_date } : {}),
    };
  return { state: 'missing', monitored: true };
}

/** The status of one catalog item or Radarr/Sonarr-only title. */
export function statusOf(s, item) {
  const admin = viewer(s).role === 'admin';
  if (item.kind === 'movie') {
    // Only titles matched to Radarr have a status, as on the server.
    if (!item.metadata?.tmdb_id) return null;
    const monitored = item.monitored !== false;
    return collapse(
      item.available === false
        ? { state: monitored ? 'missing' : 'unmonitored', monitored }
        : { state: 'available', monitored },
      admin,
    );
  }
  if (item.kind === 'episode') {
    const season = s.library.find((entry) => entry.id === item.parent_id);
    const episode = seasonsOf(s, season.parent_id)
      .flatMap((entry) => entry.episodes)
      .find((entry) => entry.id === item.id);
    return collapse(episode ? episodeStatus(episode) : null, admin);
  }
  if (item.kind === 'season') {
    const season = seasonsOf(s, item.parent_id).find(
      (entry) => entry.id === item.id,
    );
    return collapse(season ? aggregate(season.episodes) : null, admin);
  }
  if (item.kind === 'show')
    return collapse(
      aggregate(
        seasonsOf(s, item.id).flatMap((season) => season.episodes),
        item.monitored !== false,
      ),
      admin,
    );
  return null;
}

function virtualMovieStatus(s, movie) {
  const admin = viewer(s).role === 'admin';
  const status =
    movie.state === 'upcoming'
      ? { state: 'upcoming', release: now() + movie.release }
      : ['downloading', 'queued'].includes(movie.state)
        ? download(movie)
        : { state: movie.state };
  return collapse({ ...status, monitored: true }, admin);
}

function virtualSeriesStatus(s, series) {
  return collapse(
    aggregate(seasonsOf(s, series)[0].episodes),
    viewer(s).role === 'admin',
  );
}

/** Radarr/Sonarr titles that have no library item, as browse cards. */
export function virtualCards(s, kind, query) {
  const cards =
    kind === 'movie'
      ? s.titles.movies.map((movie, i) => ({
          id: `tmdb:${movie.tmdb}`,
          kind: 'movie',
          title: movie.title,
          year: movie.year,
          available: false,
          virtual: true,
          watched: false,
          artwork_url: art('movie', 30 + i),
          metadata: {},
          status: virtualMovieStatus(s, movie),
        }))
      : kind === 'show'
        ? s.titles.series.map((series, i) => ({
            id: `tmdb:${series.tmdb}`,
            kind: 'show',
            title: series.title,
            year: series.year,
            available: false,
            virtual: true,
            watched: false,
            artwork_url: art('show', 40 + i),
            metadata: {},
            status: virtualSeriesStatus(s, series),
          }))
        : [];
  return query
    ? cards.filter((card) =>
        card.title.toLowerCase().includes(query.toLowerCase()),
      )
    : cards;
}

function managerTitle(item, index, kind) {
  return {
    title: item.title,
    year: item.year,
    overview:
      item.overview ??
      'A remote observatory receives an unexpected signal. Its crew follows the clues across a landscape transformed by the changing seasons.',
    runtime: kind === 'movie' ? 110 : 45,
    genres: ['Drama', 'Adventure'],
    tmdb_id: item.tmdb ?? item.metadata?.tmdb_id ?? null,
    tvdb_id: item.tvdb ?? item.metadata?.tvdb_id ?? null,
    imdb_id: null,
    poster_url: art(kind === 'movie' ? 'movie' : 'show', index),
    backdrop_url: art('video', index),
  };
}

export async function titlesApi(s, c) {
  const { path, query, json, fail } = c;
  if (path === '/catalog/lookup') {
    const kind = query.get('kind');
    const ref = query.get('ref') ?? '';
    const match = ref.match(/^(tmdb|tvdb):(\d+)$/);
    if (!['movie', 'show'].includes(kind) || !match)
      return fail(400, 'Expected a movie or show reference');
    const key = match[1] === 'tmdb' ? 'tmdb_id' : 'tvdb_id';
    const id = Number(match[2]);
    const item = s.library.find(
      (entry) => entry.kind === kind && entry.metadata?.[key] === id,
    );
    if (item) {
      const index = Number(item.id.split('-')[1]);
      return json({
        media_id: item.id,
        title: managerTitle(item, index, kind),
        status: statusOf(s, item),
      });
    }
    const list = kind === 'movie' ? s.titles.movies : s.titles.series;
    const index = list.findIndex((entry) => entry[match[1]] === id);
    if (index < 0) return json({ media_id: null, title: null, status: null });
    const entry = list[index];
    return json({
      media_id: null,
      title: managerTitle(entry, (kind === 'movie' ? 30 : 40) + index, kind),
      status:
        kind === 'movie'
          ? virtualMovieStatus(s, entry)
          : virtualSeriesStatus(s, entry),
    });
  }
  const seasons = path.match(/^\/catalog\/([^/]+)\/seasons$/);
  if (seasons) {
    const id = decodeURIComponent(seasons[1]);
    const ref = id.match(/^(tmdb|tvdb):(\d+)$/);
    const show = ref
      ? s.titles.series.find((entry) => entry[ref[1]] === Number(ref[2]))
      : s.library.find((entry) => entry.id === id && entry.kind === 'show')
        ? id
        : null;
    if (!show) return fail(404, 'Series not found');
    const admin = viewer(s).role === 'admin';
    return json({
      seasons: seasonsOf(s, show).map(({ episodes, ...season }) => ({
        ...season,
        status: collapse(aggregate(episodes), admin),
        episodes: episodes.map((episode) => {
          const shown = { ...episode };
          delete shown.raw;
          return { ...shown, status: collapse(episodeStatus(episode), admin) };
        }),
      })),
    });
  }
  return false;
}
