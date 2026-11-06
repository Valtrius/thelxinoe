import type { FollowedPlaylist, MediaChoice } from '../playback';
import { trackQueue, type MusicItem } from './library';

export type PlaylistSort = 'manual' | 'added' | 'title' | 'artist';
export type SortDirection = 'asc' | 'desc';
export type PlaylistTrack = MusicItem & { added_at?: number };

const collator = new Intl.Collator(undefined, {
  sensitivity: 'base',
  numeric: true,
});

/** A list's tracks as they show. Ties keep their manual order. */
export function sortedPlaylistTracks<T extends PlaylistTrack>(
  tracks: T[],
  mode: PlaylistSort,
  direction: SortDirection,
): T[] {
  if (mode === 'manual') return tracks;
  const sign = direction === 'asc' ? 1 : -1;
  const compare = (left: T, right: T) => {
    if (mode === 'added')
      return sign * ((left.added_at ?? 0) - (right.added_at ?? 0));
    if (mode === 'title')
      return sign * collator.compare(left.title, right.title);
    // Tracks without an artist stay last, whichever way this sorts.
    if (!left.artist || !right.artist)
      return Number(!left.artist) - Number(!right.artist);
    return sign * collator.compare(left.artist, right.artist);
  };
  return [...tracks].sort(compare);
}

/** A list's playable tracks as a queue that follows the playlist. */
export function playlistQueue(
  { id, revision }: FollowedPlaylist,
  tracks: PlaylistTrack[],
  start?: PlaylistTrack,
): MediaChoice | null {
  const queue = trackQueue(tracks, start);
  return queue && { ...queue, playlist: { id, revision } };
}
