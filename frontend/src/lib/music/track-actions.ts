import { api } from '../api';
import { showToast } from '../providers/toasts';
import { plural, trackChoice, type MusicItem } from './library';
import { musicPlayer } from './player.svelte';
import { playlists, type Playlist } from './playlists.svelte';

export function queueTrack(track: MusicItem, next: boolean) {
  musicPlayer.controls?.insert([trackChoice(track)], next);
}

/** The playlists this viewer may add to, marking those holding `track`. */
export async function addablePlaylists(track?: string) {
  try {
    const { items } = await api<{ items: Playlist[] }>(
      track ? `/playlists?track=${encodeURIComponent(track)}` : '/playlists',
    );
    return items.filter((list) => list.can_add);
  } catch {
    return [];
  }
}

export async function addToPlaylist(
  list: Pick<Playlist, 'id' | 'name'>,
  tracks: MusicItem[],
) {
  const key = `playlist:${list.id}`;
  try {
    const added = await playlists.addTracks(list.id, tracks);
    showToast({
      key,
      tone: 'success',
      title: list.name,
      message:
        tracks.length === 1 && added
          ? `Added ${tracks[0].title}.`
          : `Added ${plural(added, 'track')}.`,
    });
  } catch (e) {
    showToast({
      key,
      tone: 'error',
      title: list.name,
      message: e instanceof Error ? e.message : String(e),
    });
  }
}
