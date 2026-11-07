import { formatClock } from '../providers/utils';
import type { Episode, Season } from './catalog';

/**
 * Where an episode stands. Only administrators see `missing` and `unmonitored`;
 * other users get `unavailable` for both.
 */
export type EpisodePhase =
  | 'available'
  | 'downloading'
  | 'missing'
  | 'unmonitored'
  | 'unavailable'
  | 'upcoming';

const seconds = () => Date.now() / 1000;
const airs = (episode: Episode, now: number) =>
  episode.air_date !== null && episode.air_date > now;

export function phase(episode: Episode, now = seconds()): EpisodePhase {
  const state = episode.status?.state;
  if (episode.available || state === 'available') return 'available';
  if (state === 'downloading' || state === 'queued') return 'downloading';
  if (state === 'upcoming' || airs(episode, now)) return 'upcoming';
  if (state === 'missing' || state === 'unmonitored') return state;
  return 'unavailable';
}

/** An aired episode that is neither here nor coming: what administrators search for. */
export const wanted = (episode: Episode, now = seconds()) =>
  ['missing', 'unmonitored'].includes(phase(episode, now));

export type SeasonSummary = {
  total: number;
  watched: number;
  available: number;
  downloading: number;
  /** Administrators' missing and unmonitored episodes. */
  missing: number;
  unavailable: number;
  upcoming: number;
  /** When its first episode airs, while none has aired yet. */
  premiere: number | null;
};

export function summarize(season: Season, now = seconds()): SeasonSummary {
  const summary: SeasonSummary = {
    total: season.episodes.length,
    watched: 0,
    available: 0,
    downloading: 0,
    missing: 0,
    unavailable: 0,
    upcoming: 0,
    premiere: null,
  };
  for (const episode of season.episodes) {
    if (episode.watched) summary.watched++;
    const current = phase(episode, now);
    if (current === 'unmonitored') summary.missing++;
    else summary[current]++;
  }
  if (summary.total && summary.upcoming === summary.total)
    summary.premiere =
      season.episodes
        .map((episode) => episode.air_date)
        .filter((date): date is number => date !== null)
        .sort((a, b) => a - b)[0] ?? null;
  return summary;
}

/** How much of a season is on disk, for its tab. */
export type SeasonFiles =
  | 'complete'
  /** Everything aired is here; more is still to come. */
  | 'ongoing'
  | 'upcoming'
  | 'downloading'
  | 'partial'
  | 'empty';

export function seasonFiles({
  total,
  available,
  downloading,
  upcoming,
}: SeasonSummary): SeasonFiles {
  if (downloading) return 'downloading';
  if (total && upcoming === total) return 'upcoming';
  if (upcoming && available + upcoming === total) return 'ongoing';
  if (!available) return 'empty';
  return available === total ? 'complete' : 'partial';
}

/** Regular episodes in viewing order. Specials stand outside it. */
const inOrder = (seasons: Season[]) =>
  seasons
    .filter((season) => season.number > 0)
    .flatMap((season) => season.episodes);
const afterLastWatched = (episodes: Episode[]) =>
  episodes.slice(
    episodes.map((episode) => episode.watched).lastIndexOf(true) + 1,
  );
const playable = (episode: Episode) => !!episode.id && episode.available;
/** The one to go on with: in progress, else the first unwatched. */
function continuing(episodes: Episode[]) {
  const unwatched = episodes.filter(
    (episode) => playable(episode) && !episode.watched,
  );
  return unwatched.find((episode) => episode.position) ?? unwatched[0];
}

/** The episode Play starts: like the server's Next up, it follows the last watched. */
export function episodeToPlay(seasons: Season[]) {
  const episodes = inOrder(seasons);
  return (
    continuing(afterLastWatched(episodes)) ??
    continuing(episodes) ??
    episodes.find(playable)
  );
}

/** Where a series opens: where watching goes on, even when that file isn't here yet. */
export function seasonToOpen(seasons: Season[]) {
  const episodes = inOrder(seasons);
  const later = afterLastWatched(episodes);
  return (
    (continuing(later) ?? later[0] ?? episodes.at(-1))?.season ??
    seasons[0]?.number ??
    null
  );
}

export type EpisodeFilter = 'all' | 'unwatched' | 'missing';
export const shown = (
  episode: Episode,
  filter: EpisodeFilter,
  now = seconds(),
) =>
  filter === 'all' ||
  (filter === 'unwatched' ? !episode.watched : wanted(episode, now));

/** Where an unfinished episode resumes, or '' when it has not been started. */
export function resumeText(episode: Episode) {
  const { position, duration } = episode;
  if (episode.watched || !position || !duration || position >= duration)
    return '';
  const left = Math.max(1, Math.round((duration - position) / 60));
  return `Resume at ${formatClock(position)} · ${left} min left`;
}

/** S02E06 */
export const episodeCode = (episode: Pick<Episode, 'season' | 'number'>) =>
  `S${String(episode.season).padStart(2, '0')}E${String(episode.number).padStart(2, '0')}`;
