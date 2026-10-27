import { api, serverUrl } from '../api';
import type { Probe } from './media-facts';
import type { TitleStatus } from './status';

export type CatalogFile = {
  id: string;
  edition: string;
  present: boolean;
  probe: Probe;
};
export type CatalogItem = {
  id: string;
  kind: string;
  title: string;
  year: number | null;
  overview?: string | null;
  parent_id?: string | null;
  sort_number?: number | null;
  available: boolean;
  metadata: Record<string, unknown>;
  artwork_url?: string | null;
  backdrop_url?: string | null;
  status?: TitleStatus | null;
  keep?: boolean;
  /** The season or series whose Keep covers this item (administrators only). */
  kept_by?: 'show' | 'season' | null;
  /** Kept seasons and episodes inside this item (administrators only). */
  kept_within?: number;
  files?: CatalogFile[];
  watched?: boolean;
  virtual?: boolean;
  /** A series no more episodes will air for. */
  ended?: boolean;
};
export type Flags = {
  favorite: boolean;
  watch_later: boolean;
  watched: boolean;
};
export type Episode = {
  id: string | null;
  season: number;
  number: number;
  title: string;
  overview: string | null;
  air_date: number | null;
  runtime: number | null;
  available: boolean;
  watched: boolean;
  position: number | null;
  duration: number | null;
  status: TitleStatus | null;
  /** Pictures of the episode, best first. */
  stills: string[];
  /** Its Sonarr episode, for administrators. */
  manager_id?: number | null;
  /** Sonarr's file of the episode, for administrators. */
  file?: { quality: string | null; size: number | null } | null;
};
export type Season = {
  number: number;
  id: string | null;
  title: string;
  status: TitleStatus | null;
  watched: boolean;
  episodes: Episode[];
};
export type ManagerTitle = {
  title: string;
  year: number | null;
  overview: string | null;
  runtime: number | null;
  genres: string[];
  tmdb_id: number | null;
  tvdb_id: number | null;
  imdb_id: string | null;
  ended: boolean;
  poster_url: string | null;
  backdrop_url: string | null;
};
export type Lookup = {
  media_id: string | null;
  title: ManagerTitle | null;
  status: TitleStatus | null;
};
export type TitleKind = 'movie' | 'show';

const path = (id: string) => `/catalog/${encodeURIComponent(id)}`;

export const loadItem = (id: string) => api<CatalogItem>(path(id));
export const loadSeasons = (id: string) =>
  api<{ seasons: Season[] }>(`${path(id)}/seasons`);
export const lookup = (kind: TitleKind, ref: string) =>
  api<Lookup>(`/catalog/lookup?kind=${kind}&ref=${encodeURIComponent(ref)}`);
export const loadFlags = (id: string) => api<Flags>(`${path(id)}/state`);
export const setFlag = (id: string, key: keyof Flags, value: boolean) =>
  api<Flags>(`${path(id)}/state`, 'PUT', { [key]: value });
/** Mark a season's episodes, or with `through` an episode and every one before it. */
export const setWatched = (id: string, watched: boolean, through = false) =>
  api<{ episodes: number }>(`${path(id)}/watched`, 'PUT', {
    watched,
    through,
  });

/**
 * Why Keep stops file changes to this item, or '' when nothing does. Keep on the
 * item, on its season or series, or on anything inside it blocks deleting and
 * monitoring changes.
 */
export function keptReason(item: CatalogItem) {
  const scope = item.kind === 'show' ? 'series' : item.kind;
  if (item.kept_by)
    return `Keep is on for the whole ${item.kept_by === 'show' ? 'series' : 'season'}. Turn it off there first.`;
  if (item.keep) return 'Keep is on. Turn it off first.';
  const inside = item.kept_within ?? 0;
  if (inside)
    return `${inside} ${item.kind === 'season' ? (inside === 1 ? 'episode' : 'episodes') : inside === 1 ? 'season or episode' : 'seasons or episodes'} in this ${scope} ${inside === 1 ? 'is' : 'are'} kept. Turn off Keep there first.`;
  return '';
}

/** An episode named after its file repeats the series name; leave it out. */
export const episodeLabel = (title: string, show: string) =>
  show && title.startsWith(`${show} · `) ? title.slice(show.length + 3) : title;

/** Library artwork is served by Thelxinoe; TMDB artwork is absolute. */
export const asset = (url?: string | null) =>
  url ? (/^https:\/\//.test(url) ? url : `${serverUrl()}${url}`) : '';

export const metadataNumber = (item: CatalogItem | null, key: string) => {
  const value = item?.metadata?.[key];
  return typeof value === 'number' && value > 0 ? value : undefined;
};

/** Walk from a season or episode to its series. */
export async function seriesOf(item: CatalogItem) {
  let current = item;
  const chain: CatalogItem[] = [];
  while (current.kind !== 'show' && current.parent_id) {
    chain.push(current);
    current = await loadItem(current.parent_id);
  }
  return { show: current, chain };
}

// Administrator actions. Monitor and delete are two-step operations on the
// server: the first call captures the files, the second applies the change.
export const setKeep = (id: string, keep: boolean) =>
  api(`/admin/media/${encodeURIComponent(id)}/keep`, 'PUT', { keep });
export const prepareOperation = (
  id: string,
  action: 'monitor' | 'unmonitor' | 'delete',
) =>
  api<{ id: string; files: number }>('/admin/media/operations', 'POST', {
    media_id: id,
    action,
  });
export const executeOperation = (operation: string) =>
  api(`/admin/media/operations/${operation}/execute`, 'POST');
export const refreshMetadata = (id: string) =>
  api(`${path(id)}/refresh`, 'POST');

export type LibraryRoot = {
  id: string;
  name: string;
  kind: string;
  path: string;
  last_scan: number | null;
  scan_error: string | null;
  watching?: boolean | null;
};
export const loadRoots = () =>
  api<{ items: LibraryRoot[] }>('/catalog/roots').then((r) => r.items);
export const scanRoot = (id: string) =>
  api(`/catalog/roots/${encodeURIComponent(id)}/scan`, 'POST');
