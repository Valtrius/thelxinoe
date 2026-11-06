import { describe, expect, it } from 'vitest';
import type { MusicItem } from './library';
import {
  playlistQueue,
  sortedPlaylistTracks,
  type PlaylistTrack,
} from './playlist-order';

const track = (
  id: string,
  title: string,
  artist: string | null,
  added_at: number,
  available = true,
): PlaylistTrack =>
  ({
    id,
    kind: 'track',
    title,
    year: null,
    available,
    metadata: {},
    artist,
    added_at,
  }) satisfies MusicItem & { added_at: number };

const tracks = [
  track('a', 'Zebra', 'beta', 30),
  track('b', 'apple', null, 10),
  track('c', 'Track 10', 'Alpha', 20),
  track('d', 'Track 9', 'alpha', 20),
];
const ids = (list: PlaylistTrack[]) => list.map((item) => item.id).join('');

describe('sortedPlaylistTracks', () => {
  it('keeps the manual order', () => {
    expect(sortedPlaylistTracks(tracks, 'manual', 'desc')).toBe(tracks);
  });

  it('sorts by date added with ties in manual order', () => {
    expect(ids(sortedPlaylistTracks(tracks, 'added', 'asc'))).toBe('bcda');
    expect(ids(sortedPlaylistTracks(tracks, 'added', 'desc'))).toBe('acdb');
  });

  it('sorts titles ignoring case, reading numbers as numbers', () => {
    expect(ids(sortedPlaylistTracks(tracks, 'title', 'asc'))).toBe('bdca');
    expect(ids(sortedPlaylistTracks(tracks, 'title', 'desc'))).toBe('acdb');
  });

  it('sorts artists with the unknown one last either way', () => {
    expect(ids(sortedPlaylistTracks(tracks, 'artist', 'asc'))).toBe('cdab');
    expect(ids(sortedPlaylistTracks(tracks, 'artist', 'desc'))).toBe('acdb');
  });

  it('leaves the given list untouched', () => {
    sortedPlaylistTracks(tracks, 'title', 'asc');
    expect(ids(tracks)).toBe('abcd');
  });
});

describe('playlistQueue', () => {
  it('queues the shown order from the chosen track, skipping unavailable ones', () => {
    const shown = [
      track('a', 'A', null, 0),
      track('b', 'B', null, 0, false),
      track('c', 'C', null, 0),
    ];
    const queue = playlistQueue({ id: 'list', revision: 3 }, shown, shown[2])!;
    expect(queue.playlist).toEqual({ id: 'list', revision: 3 });
    expect(queue.id).toBe('c');
    expect(queue.queueIndex).toBe(1);
    expect(queue.queue?.map((item) => item.id)).toEqual(['a', 'c']);
  });

  it('has nothing to play in an empty list', () => {
    expect(playlistQueue({ id: 'list', revision: 0 }, [])).toBeNull();
  });
});
