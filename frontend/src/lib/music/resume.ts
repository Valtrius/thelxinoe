import { serverUrl } from '../api';
import type { MusicState } from '../gapless';
import type { MediaChoice } from '../playback';

const key = (user: string) => `thelxinoe:${serverUrl()}:${user}:music`;

export function forgetMusic(user: string) {
  localStorage.removeItem(key(user));
}
export function saveMusic(user: string, state: MusicState) {
  if (state.finished) return forgetMusic(user);
  // A track being read has no position yet: the last one saved still holds.
  if (state.loading) return;
  const { queue, index, position, duration, repeat, shuffle } = state;
  try {
    localStorage.setItem(
      key(user),
      JSON.stringify({ queue, index, position, duration, repeat, shuffle }),
    );
  } catch {
    forgetMusic(user);
  }
}
/** The queue this device left open, ready to play on from where it stopped. */
export function savedMusic(user: string): MediaChoice | null {
  try {
    const saved = JSON.parse(localStorage.getItem(key(user)) ?? 'null');
    const track: MediaChoice | undefined = saved?.queue?.[saved.index];
    if (!track) return null;
    return {
      ...track,
      position: saved.position,
      duration: saved.duration || track.duration,
      queueIndex: saved.index,
      queue: saved.queue,
      parked: { repeat: saved.repeat, shuffled: saved.shuffle },
    };
  } catch {
    return null;
  }
}
