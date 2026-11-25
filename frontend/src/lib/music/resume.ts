import { serverUrl } from '../api';
import type { MusicState } from '../gapless';
import type { MediaChoice } from '../playback';
import { savedChoice, type Queue } from './queue';

const key = (user: string) => `thelxinoe:${serverUrl()}:${user}:music`;

export function forgetMusic(user: string) {
  localStorage.removeItem(key(user));
}
export function saveMusic(user: string, state: MusicState, queue: Queue) {
  if (state.finished) return forgetMusic(user);
  // A track being read has no position yet: the last one saved still holds.
  if (state.loading) return;
  try {
    localStorage.setItem(
      key(user),
      JSON.stringify(queue.save(state.position, state.duration)),
    );
  } catch {
    forgetMusic(user);
  }
}
/** The Queue this device left open, ready to play on from where it stopped. */
export function savedMusic(user: string): MediaChoice | null {
  try {
    return savedChoice(JSON.parse(localStorage.getItem(key(user)) ?? 'null'));
  } catch {
    return null;
  }
}
