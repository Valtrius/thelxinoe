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
export const episodeCode = (episode: Episode) =>
  `S${String(episode.season).padStart(2, '0')}E${String(episode.number).padStart(2, '0')}`;
