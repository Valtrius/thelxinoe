// Radarr/Sonarr state the server caches for each title. Regular users never see
// `missing`, `unmonitored` or `monitored`; the server collapses them to
// `unavailable` before replying.
export type TitleState =
  | 'available'
  | 'partial'
  | 'downloading'
  | 'queued'
  | 'upcoming'
  | 'missing'
  | 'unmonitored'
  | 'unavailable';

export type TitleStatus = {
  state: TitleState;
  progress?: number;
  eta?: number;
  release?: number;
  episodes?: { available: number; total: number };
  downloading?: number;
  monitored?: boolean;
};

export type StatusTone = 'success' | 'accent' | 'warning' | 'muted';

// A movie or series known only to Radarr/Sonarr is addressed by its TMDB or
// TVDB id instead of a library id.
export const isTitleRef = (id: string) => /^(tmdb|tvdb):\d+$/.test(id);

function yearIn(date: Date, timezone: string) {
  return new Intl.DateTimeFormat('en', {
    timeZone: timezone,
    year: 'numeric',
  }).format(date);
}

function day(seconds: number, timezone: string, now: number) {
  const date = new Date(seconds * 1000);
  return new Intl.DateTimeFormat(undefined, {
    timeZone: timezone,
    day: 'numeric',
    month: 'short',
    ...(yearIn(date, timezone) !== yearIn(new Date(now), timezone)
      ? { year: 'numeric' }
      : {}),
  }).format(date);
}

function remaining(seconds: number, now: number) {
  const minutes = Math.max(1, Math.round((seconds * 1000 - now) / 60000));
  if (minutes < 60) return `${minutes} min left`;
  const hours = Math.round(minutes / 60);
  return hours < 48 ? `${hours} h left` : `${Math.round(hours / 24)} days left`;
}

const percent = (progress = 0) =>
  `${Math.min(99, Math.floor(progress * 100))}%`;

const tones: Record<TitleState, StatusTone> = {
  available: 'success',
  partial: 'accent',
  downloading: 'accent',
  queued: 'muted',
  upcoming: 'accent',
  missing: 'warning',
  unmonitored: 'muted',
  unavailable: 'muted',
};

/** The status line of a title page or episode row. */
export function statusText(
  status: TitleStatus,
  timezone: string,
  now = Date.now(),
): { label: string; detail?: string; tone: StatusTone } {
  const next = status.release ? day(status.release, timezone, now) : undefined;
  const tone = tones[status.state];
  switch (status.state) {
    case 'available':
      return {
        label: 'Available',
        detail: next && status.episodes ? `Next episode ${next}` : undefined,
        tone,
      };
    case 'partial':
      return {
        label: `${status.episodes?.available ?? 0} of ${status.episodes?.total ?? 0} episodes`,
        detail: next ? `Next episode ${next}` : undefined,
        tone,
      };
    case 'downloading': {
      const episodes =
        status.downloading && status.downloading > 1
          ? `${status.downloading} episodes · `
          : '';
      if ((status.progress ?? 0) >= 1)
        return { label: `Importing ${episodes}`.trim(), tone };
      return {
        label: `Downloading ${episodes}${percent(status.progress)}`,
        detail: status.eta ? remaining(status.eta, now) : undefined,
        tone,
      };
    }
    case 'queued':
      return { label: 'Queued', tone };
    case 'upcoming':
      return { label: next ? `Coming ${next}` : 'Coming soon', tone };
    case 'missing':
      return { label: 'Missing', tone };
    case 'unmonitored':
      return { label: 'Not monitored', tone };
    case 'unavailable':
      return { label: 'Not available', tone };
  }
}

/** A short chip for a poster. Available titles need none. */
export function statusChip(
  status: TitleStatus | null | undefined,
  timezone: string,
  now = Date.now(),
): { label: string; tone: StatusTone } | null {
  if (!status || status.state === 'available') return null;
  const tone = tones[status.state];
  switch (status.state) {
    case 'partial':
      return {
        label: `${status.episodes?.available ?? 0}/${status.episodes?.total ?? 0}`,
        tone,
      };
    case 'downloading':
      return {
        label:
          (status.progress ?? 0) >= 1 ? 'Importing' : percent(status.progress),
        tone,
      };
    case 'upcoming':
      return {
        label: status.release ? day(status.release, timezone, now) : 'Soon',
        tone,
      };
    default:
      return { label: statusText(status, timezone, now).label, tone };
  }
}
