import { libraryArtworkSet } from '../artwork';
import type { MediaChoice } from '../playback';
import { artwork } from './library';

type Actions = {
  play: () => void;
  pause: () => void;
  previous: () => void;
  next: () => void;
  seek: (to: number) => void;
};

/** Tells the system what the music dock plays and takes presses on its controls. */
export class MusicSession {
  private session = navigator.mediaSession as MediaSession | undefined;
  private handlers: MediaSessionAction[] = [];
  private shown: {
    paused: boolean;
    position: number;
    duration: number;
    at: number;
  } | null = null;
  constructor(actions: Actions) {
    const handlers: [MediaSessionAction, MediaSessionActionHandler][] = [
      ['play', actions.play],
      ['pause', actions.pause],
      ['previoustrack', actions.previous],
      ['nexttrack', actions.next],
      [
        'seekto',
        ({ seekTime }) => {
          if (seekTime !== undefined) actions.seek(seekTime);
        },
      ],
    ];
    for (const [action, handler] of handlers)
      try {
        this.session?.setActionHandler(action, handler);
        this.handlers.push(action);
      } catch {
        // A browser without this action leaves its button out.
      }
  }
  track(track: MediaChoice) {
    if (!this.session) return;
    const source = artwork(track.artwork_url);
    const sized = source && libraryArtworkSet(source)?.split(' ')[0];
    this.session.metadata = new MediaMetadata({
      title: track.title,
      artist: track.artist ?? '',
      album: track.album ?? '',
      artwork: sized
        ? [{ src: sized, sizes: '500x500' }]
        : source
          ? [{ src: source }]
          : [],
    });
  }
  /**
   * The system's seek bar runs on its own from what it was last told, so it
   * only hears of a change: a new track, a pause, or a jump in position.
   */
  progress(paused: boolean, position: number, duration: number) {
    const { session, shown } = this;
    if (!session) return;
    session.playbackState = paused ? 'paused' : 'playing';
    const at = performance.now();
    const expected = shown
      ? shown.position + (shown.paused ? 0 : (at - shown.at) / 1000)
      : 0;
    if (
      shown?.paused === paused &&
      shown.duration === duration &&
      Math.abs(position - expected) < 2
    )
      return;
    this.shown = { paused, position, duration, at };
    session.setPositionState?.(
      duration > 0
        ? { duration, position: Math.min(position, duration), playbackRate: 1 }
        : undefined,
    );
  }
  close() {
    if (!this.session) return;
    for (const action of this.handlers)
      this.session.setActionHandler(action, null);
    this.session.metadata = null;
    this.session.playbackState = 'none';
    this.session.setPositionState?.();
  }
}
