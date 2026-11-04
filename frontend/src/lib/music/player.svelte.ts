import type { MusicState } from '../gapless';
import type { MediaChoice } from '../playback';

/** What the music dock plays, for pages that mark the playing track and add to its queue. */
class MusicPlayer {
  state = $state<MusicState | null>(null);
  current = $state<MediaChoice | null>(null);
  /** Present while a queue exists. */
  controls = $state<{
    insert: (items: MediaChoice[], next: boolean) => void;
  } | null>(null);
  /** Silences the dock as it starts to leave, before it is removed. */
  stop: (() => void) | null = null;
}
export const musicPlayer = new MusicPlayer();
