import { api } from '../api';
import type { RequestAttention } from '../attention';

export type RequestState =
  | 'pending'
  | 'approved'
  | 'denied'
  | 'cancelled'
  | 'adding'
  | 'searching'
  | 'requested'
  | 'available'
  | 'failed'
  | 'uncertain';

export type Album = {
  /** The album's MusicBrainz release group. */
  external_id: string;
  title: string;
  artist?: string | null;
  /** The artist's MusicBrainz ID. */
  artist_id?: string | null;
  year?: number | null;
  date?: string | null;
  /** Album, EP, Live, Compilation… */
  type?: string | null;
  track_count?: number | null;
  /** A same-origin path; the server relays cover art. */
  cover?: string | null;
  in_library?: boolean;
  /** Lidarr monitors it but has no files yet. */
  missing?: boolean;
  /** Lidarr's metadata profile keeps it, so it can be requested. */
  requestable?: boolean;
  request?: { id: string; state: RequestState } | null;
};

export type AlbumDetail = Album & {
  label?: string | null;
  format?: string | null;
  edition?: string | null;
  /** Seconds. */
  duration?: number | null;
  tracks: {
    disc: number;
    number: number;
    title: string;
    length?: number | null;
  }[];
};

export type Artist = {
  /** The artist's MusicBrainz ID. */
  id: string;
  name: string;
  type?: string | null;
  disambiguation?: string | null;
  overview?: string | null;
  genres: string[];
  image: string;
  in_lidarr?: boolean;
  /** Lidarr monitors the artist's new releases. */
  monitored?: boolean;
  request?: { id: string; state: RequestState } | null;
  /** The library artist, when the library has one. */
  library_id?: string | null;
  album_count?: number;
};

export type ArtistView = {
  artist: Artist;
  albums: Album[];
  other: Album[];
  profile: string;
};

export type MusicSearch = {
  /** The artist the search names exactly, with its albums. */
  artist: Artist | null;
  artists: Artist[];
  albums: Album[];
  matches: Album[];
  excluded: Album[];
};

export type MusicService = {
  id: string;
  name: string;
  kind: string;
  ready: boolean;
};

export type MusicRequest = {
  id: string;
  title: string;
  state: RequestState;
  created_at: number;
  error?: string | null;
  username: string;
  user_id: string;
  kind: string;
  external_id: string;
  /** 'artist' asks Lidarr to monitor the artist's new releases. */
  target: 'item' | 'artist';
  artist?: string | null;
  artist_id?: string | null;
  attention?: RequestAttention | null;
};

// A declined, cancelled or failed request leaves the album requestable again.
export const requestable = (state?: RequestState) =>
  !state || ['denied', 'cancelled', 'failed'].includes(state);

export const requestLabel = (state: RequestState) =>
  ({
    pending: 'Pending approval',
    approved: 'Requested',
    adding: 'Requested',
    searching: 'Requested',
    requested: 'Requested',
    available: 'Available',
    failed: 'Failed',
    uncertain: 'Needs attention',
    denied: 'Declined',
    cancelled: 'Cancelled',
  })[state];

/** Whether an album can be requested now. */
export const canRequest = (album: Album) =>
  album.requestable !== false &&
  !album.in_library &&
  !album.missing &&
  requestable(album.request?.state);

/** How a request to monitor an artist's new releases stands. */
export const monitorLabel = (state: RequestState) =>
  state === 'pending'
    ? 'Monitoring pending approval'
    : ['approved', 'adding', 'searching', 'requested', 'available'].includes(
          state,
        )
      ? 'Monitoring new releases'
      : requestLabel(state);

export const cover = (path?: string | null) =>
  path && path.startsWith('/') && !path.startsWith('//') ? path : '';

export const albumCover = (id: string) =>
  `/api/v1/acquisition/music/covers/${encodeURIComponent(id)}`;
export const artistImage = (id: string) =>
  `/api/v1/acquisition/music/artists/${encodeURIComponent(id)}/image`;

/** The Lidarr service music requests go to, ready or not; null without one. */
export async function lidarr() {
  const { items } = await api<{ items: MusicService[] }>(
    '/acquisition/services',
  );
  const services = items.filter((item) => item.kind === 'lidarr');
  return services.find((item) => item.ready) ?? services[0] ?? null;
}

export function requestMusic(
  service: MusicService,
  external_id: string,
  target: 'item' | 'artist' = 'item',
) {
  return api<{ id: string; state: RequestState }>(
    '/acquisition/requests',
    'POST',
    { service_id: service.id, external_id, target },
  );
}

export const scoped = (path: string, service: MusicService) =>
  `${path}${path.includes('?') ? '&' : '?'}service_id=${encodeURIComponent(service.id)}`;

/** Albums sharing `id` in `lists` take on the request. */
export function markRequested(
  lists: Album[][],
  id: string,
  request: { id: string; state: RequestState },
) {
  for (const list of lists)
    for (const album of list)
      if (album.external_id === id) album.request = request;
}
