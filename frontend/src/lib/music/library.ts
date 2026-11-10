import { api, serverUrl } from '../api';
import { inBackground } from '../page-readiness';
import type { MediaChoice } from '../playback';

/** A music item as the catalog lists it. */
export type MusicItem = {
  id: string;
  kind: string;
  title: string;
  year: number | null;
  available: boolean;
  watched?: boolean;
  overview?: string | null;
  parent_id?: string | null;
  sort_number?: number | null;
  artwork_url?: string;
  metadata: Record<string, unknown>;
  /** Seconds: a track's length, or everything in an album or artist. */
  duration?: number;
  track_count?: number;
  album_count?: number;
  artist?: string | null;
  album?: string | null;
  artist_id?: string | null;
  audio?: {
    codec: string | null;
    sample_rate: number | null;
    bit_depth: number | null;
  } | null;
};
export type MusicPage = { items: MusicItem[]; total: number };

export const catalogQuery = (
  params: Record<string, string | number | undefined>,
) =>
  `/catalog?${new URLSearchParams(
    Object.entries(params)
      .filter(([, value]) => value !== undefined && value !== '')
      .map(([key, value]) => [key, String(value)]),
  )}`;

/** Every page of a list, for queues that play a whole album or artist. */
export async function listAll(
  path: string,
  read: (page: string) => Promise<MusicPage> = (page) => api<MusicPage>(page),
) {
  const items: MusicItem[] = [];
  for (;;) {
    const page = await read(`${path}&offset=${items.length}&limit=500`);
    items.push(...page.items);
    // A list without a total is not paged.
    if (!page.items.length || !(items.length < page.total)) return items;
  }
}
export const albumTracks = (album: string) =>
  listAll(catalogQuery({ parent: album }));
export const artistTracks = (artist: string) =>
  listAll(catalogQuery({ ancestor: artist, kind: 'track' }));

export function trackChoice(track: MusicItem): MediaChoice {
  return {
    id: track.id,
    title: track.title,
    kind: 'track',
    artwork_url: track.artwork_url,
    artist: track.artist ?? undefined,
    album: track.album ?? undefined,
    duration: track.duration ?? undefined,
  };
}
const playable = (track: MusicItem) =>
  track.kind === 'track' && track.available;
/** The playable tracks of a list as a queue, starting at `start` or the first. */
export function trackQueue(
  tracks: MusicItem[],
  start?: MusicItem,
): MediaChoice | null {
  const list = tracks.filter(playable);
  const index = Math.max(
    0,
    start ? list.findIndex((track) => track.id === start.id) : 0,
  );
  const first = list[index];
  if (!first) return null;
  return {
    ...trackChoice(first),
    queueIndex: index,
    queue: list.map(trackChoice),
  };
}
/** The playable tracks of a list that `queue` started without, read in the
 * background so a page opened meanwhile doesn't wait for them. */
export async function restOfList(
  path: string,
  queue: MediaChoice[],
  signal: AbortSignal,
) {
  const all = await listAll(path, (page) =>
    inBackground(() => api<MusicPage>(page, 'GET', undefined, signal)),
  );
  return restOf(all, queue);
}
/** The playable tracks of a list that `queue` started without. */
export function restOf(all: MusicItem[], queue: MediaChoice[]) {
  const known = new Set(queue.map((choice) => choice.id));
  return all
    .filter((track) => playable(track) && !known.has(track.id))
    .map(trackChoice);
}
export function shuffled<T>(items: T[]): T[] {
  const result = [...items];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

/** Library artwork is served by Thelxinoe; other artwork is absolute. */
export const artwork = (url?: string | null) =>
  url ? (/^https?:\/\//.test(url) ? url : `${serverUrl()}${url}`) : '';

/** A length in hours and minutes, for albums and artists. */
export function length(seconds?: number | null) {
  if (!seconds || seconds < 30) return '';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${Math.max(1, minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}
export function audioFormat(audio?: MusicItem['audio'] | null) {
  if (!audio?.codec) return '';
  const rate = audio.sample_rate
    ? ` / ${(audio.sample_rate / 1000).toFixed(1).replace(/\.0$/, '')} kHz`
    : '';
  return `${audio.codec.toUpperCase()}${audio.bit_depth ? ` ${audio.bit_depth}-bit` : ''}${rate}`;
}
// The scanner numbers tracks as disc × 10000 + track.
export const trackNumber = (item: MusicItem) =>
  item.sort_number ? item.sort_number % 10000 || null : null;
export const discNumber = (item: MusicItem) =>
  item.sort_number ? Math.floor(item.sort_number / 10000) || 1 : 1;
export const plural = (count: number, word: string) =>
  `${count} ${word}${count === 1 ? '' : 's'}`;
