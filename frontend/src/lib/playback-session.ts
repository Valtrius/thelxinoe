import type Hls from 'hls.js';
import type { HlsConfig } from 'hls.js';
import { api, ApiError } from './api';
import { inBackground } from './page-readiness';
import { PlaybackActivity } from './playback-activity';
import {
  mediaUrl,
  playbackRequest,
  type MediaChoice,
  type Options,
  type Orientation,
  type Playback,
  type Preparation,
} from './playback';

export type ProgressState = 'playing' | 'paused' | 'stopped';
export type ProgressReport = {
  sequence: number;
  position: number;
  state: ProgressState;
  active_seconds?: number;
};
/** Where a transcode starts again after a seek. */
export type SeekAnswer = {
  url: string;
  timeline_start: number;
  position: number;
};
export type SessionRequest = {
  choice: MediaChoice;
  options: Options;
  /** Where playback starts; the choice's own position when absent. */
  at?: number;
  preparation: Preparation;
  orientation?: Orientation;
};
/** The server's playback endpoints. */
export type SessionApi = {
  prepare(request: SessionRequest): Promise<Playback>;
  progress(
    id: string,
    report: ProgressReport,
    signal?: AbortSignal,
  ): Promise<unknown>;
  seek(id: string, position: number): Promise<SeekAnswer>;
  keepalive(id: string): Promise<unknown>;
  end(id: string): Promise<unknown>;
};
export const serverSessions: SessionApi = {
  prepare({ choice, options, at, preparation, orientation }) {
    const body = {
      media_id: choice.id,
      file_id: choice.fileId,
      options,
      position: at ?? choice.position,
      orientation,
    };
    return playbackRequest(
      () => api<Playback>('/playback', 'POST', body),
      preparation,
    );
  },
  progress: (id, report, signal) =>
    api(`/playback/${id}/progress`, 'POST', report, signal),
  seek: (id, position) =>
    api<SeekAnswer>(`/playback/${id}/seek`, 'POST', { position }),
  keepalive: (id) => api(`/playback/${id}/keepalive`, 'POST'),
  end: (id) => api(`/playback/${id}`, 'DELETE'),
};

// The server answers for a session it no longer plays with a 4xx. A dropped
// connection or a server error is transient: playback goes on and the next
// report catches up.
function refusal(error: unknown) {
  return error instanceof ApiError && error.status < 500;
}

/** What a caller does at the session's turning points; each receives the session. */
export type SessionHooks = {
  /** Before each stream attaches; a returned promise holds the attach back. */
  attaching?: (session: PlaybackSession, autoplay: boolean) => unknown;
  /** A stream attached to play at once can start. */
  play?: (session: PlaybackSession) => void;
  /** hls.js buffered a fragment. */
  buffered?: (session: PlaybackSession) => void;
  /** A seek restarted the stream on a new timeline; it reattaches next. */
  moved?: (session: PlaybackSession) => void;
  /**
   * Restarts a failed transcode where it stopped, or returns false when it
   * can't now. With this hook a session restarts its stream once.
   */
  restart?: (session: PlaybackSession) => boolean;
  /** The stream failed and doesn't restart. */
  interrupted?: (session: PlaybackSession) => void;
  /** The server refused the session: it no longer plays it. */
  refused?: (session: PlaybackSession, error: unknown) => void;
};
export type SessionSetup = {
  hooks?: SessionHooks;
  /** Loads hls.js, which a session needs only for a playlist. */
  loadHls: () => typeof Hls | Promise<typeof Hls>;
  /** Options hls.js takes for this playback, over the shared ones. */
  hls?: (playback: Playback) => Partial<HlsConfig>;
  /** A file stream seeks to where it starts even when that is its beginning. */
  seekAtStart?: boolean;
  /** Stopping cancels the reports still pending; the stopped report gives up after 5 s. */
  abortableReports?: boolean;
  /** Requests go out only while this holds, such as until the user signs out. */
  owns?: () => boolean;
  api?: SessionApi;
};

/**
 * One playback session, from the server preparing it to its end. It numbers
 * and orders its progress reports, attaches its stream to a media element,
 * restarts a transcode where a seek lands, and maps media time to the
 * timeline. Once stopped it sends nothing more.
 */
export class PlaybackSession {
  static async open(request: SessionRequest, setup: SessionSetup) {
    const playback = await (setup.api ?? serverSessions).prepare(request);
    return new PlaybackSession(playback, setup);
  }

  /** Active wall time, reported with each progress report. */
  readonly activity = new PlaybackActivity();
  private readonly api: SessionApi;
  private readonly hooks: SessionHooks;
  private readonly owns: () => boolean;
  private sequence = 0;
  private reports: Promise<unknown> = Promise.resolve();
  private requests = new AbortController();
  private ended?: Promise<void>;
  private element?: HTMLMediaElement;
  private stream?: Hls;
  /** Bumped by every attach and detach, so an attach in flight knows it was replaced. */
  private attachments = 0;
  private restarted = false;

  constructor(
    /** What the server delivers; a seek that restarts the stream replaces it. */
    public playback: Playback,
    private readonly setup: SessionSetup,
  ) {
    this.api = setup.api ?? serverSessions;
    this.hooks = setup.hooks ?? {};
    this.owns = setup.owns ?? (() => true);
  }

  get id() {
    return this.playback.id;
  }
  get stopped() {
    return this.ended !== undefined;
  }
  /** The hls.js player of the attached stream, if it plays through one. */
  get hls() {
    return this.stream;
  }
  /** The timeline position of a time in the attached stream. */
  timeline(mediaTime: number) {
    return mediaTime + this.playback.timeline_start;
  }
  /** The time in the attached stream of a timeline position. */
  mediaTime(at: number) {
    return Math.max(0, at - this.playback.timeline_start);
  }

  /** Reports progress after the reports before it; a 'stopped' report doesn't end the session. */
  report(state: ProgressState, position: number): Promise<unknown> {
    if (this.ended) return Promise.resolve();
    return this.send(state, position);
  }

  /** Reports the session stopped and detaches its stream. */
  stop(position: number): Promise<void> {
    if (this.ended) return this.ended;
    this.activity.setActive(false);
    this.requests.abort();
    const stopped = this.send('stopped', position).then(() => {});
    this.detach();
    return (this.ended = stopped);
  }

  /** Ends a session that never played by deleting it. */
  discard(): Promise<void> {
    if (this.ended) return this.ended;
    this.detach();
    return (this.ended = this.owns()
      ? this.api.end(this.id).then(
          () => {},
          () => {},
        )
      : Promise.resolve());
  }

  /** Ends a session that never played with a bare stopped report, whose failure the caller sees. */
  abandon(): Promise<void> {
    if (this.ended) return this.ended;
    this.detach();
    return (this.ended = this.api
      .progress(this.id, {
        sequence: this.sequence++,
        position: this.playback.position,
        state: 'stopped',
      })
      .then(() => {}));
  }

  /** Tells the server the session still plays; false when it didn't hear. */
  async keepalive(): Promise<boolean> {
    if (this.ended) return true;
    if (!this.owns()) return false;
    try {
      await inBackground(() => this.api.keepalive(this.id));
      return true;
    } catch (error) {
      if (refusal(error)) this.hooks.refused?.(this, error);
      return false;
    }
  }

  /**
   * Seeks the attached stream. A direct stream moves at once; a transcode
   * starts again where the server answers, unless the session stopped or
   * `current` no longer holds by then. Resolves whether the seek happened.
   */
  async seek(
    at: number,
    options: { autoplay?: boolean; current?: () => boolean } = {},
  ): Promise<boolean> {
    if (this.ended || this.playback.live) return false;
    if (this.playback.mode === 'direct') {
      if (this.element) this.element.currentTime = at;
      return true;
    }
    const next = await this.api.seek(
      this.id,
      Math.min(at, this.playback.duration - 0.05),
    );
    if (this.ended || (options.current && !options.current())) return false;
    this.playback = { ...this.playback, ...next };
    this.hooks.moved?.(this);
    await this.reattach(next.position, options.autoplay ?? false);
    return true;
  }

  /** Plays the session's stream on `element` from timeline position `at`. */
  attach(element: HTMLMediaElement, at: number, autoplay = false) {
    if (this.ended) return Promise.resolve();
    this.element = element;
    return this.reattach(at, autoplay);
  }

  /** Stops the stream from reaching the element; the caller empties the element. */
  detach() {
    this.attachments++;
    this.stream?.destroy();
    this.stream = undefined;
    if (this.element) this.element.onloadedmetadata = null;
  }

  private send(state: ProgressState, position: number) {
    const report = {
      sequence: this.sequence++,
      position,
      state,
      active_seconds: this.activity.seconds(),
    };
    const signal = !this.setup.abortableReports
      ? undefined
      : state === 'stopped'
        ? AbortSignal.timeout(5000)
        : this.requests.signal;
    this.reports = this.reports
      .catch(() => {})
      // Progress reports never hold back the page being opened.
      .then(() =>
        this.owns()
          ? inBackground(() => this.api.progress(this.id, report, signal))
          : undefined,
      )
      .catch((error) => {
        if (refusal(error)) this.hooks.refused?.(this, error);
      });
    return this.reports;
  }

  private async reattach(at: number, autoplay: boolean) {
    const element = this.element;
    if (this.ended || !element) return;
    const attachment = ++this.attachments;
    const current = () => attachment === this.attachments && !this.ended;
    const preparing = this.hooks.attaching?.(this, autoplay);
    if (preparing) {
      await preparing;
      if (!current()) return;
    }
    this.stream?.destroy();
    this.stream = undefined;
    const { playback } = this;
    const { url } = playback;
    const from = playback.live ? undefined : this.mediaTime(at);
    const play = autoplay ? this.hooks.play : undefined;
    // Online videos play their original files as HLS, even in direct mode.
    const playlist = new URL(url, location.href).pathname.endsWith('.m3u8');
    let HlsPlayer: typeof Hls | undefined;
    if (playlist) {
      const loading = this.setup.loadHls();
      HlsPlayer = loading instanceof Promise ? await loading : loading;
      if (!current()) return;
    }
    if (
      !HlsPlayer ||
      (!HlsPlayer.isSupported() &&
        element.canPlayType('application/vnd.apple.mpegurl'))
    ) {
      element.src = mediaUrl(url);
      const seekTo = from || this.setup.seekAtStart ? from : undefined;
      element.onloadedmetadata =
        seekTo !== undefined || play
          ? () => {
              if (!current()) return;
              if (seekTo !== undefined) element.currentTime = seekTo;
              play?.(this);
            }
          : null;
      return;
    }
    const hls = new HlsPlayer({
      enableWorker: false,
      startPosition: from ?? -1,
      maxBufferLength: 30,
      ...this.setup.hls?.(playback),
    });
    this.stream = hls;
    hls.loadSource(mediaUrl(url));
    hls.attachMedia(element);
    if (play)
      hls.on(HlsPlayer.Events.MANIFEST_PARSED, () => {
        if (current()) play(this);
      });
    const { buffered } = this.hooks;
    if (buffered) hls.on(HlsPlayer.Events.FRAG_BUFFERED, () => buffered(this));
    hls.on(HlsPlayer.Events.ERROR, (_, data) => {
      if (data.fatal && current()) this.failed();
    });
  }

  private failed() {
    const { restart, interrupted } = this.hooks;
    if (
      restart &&
      !this.restarted &&
      this.playback.mode !== 'direct' &&
      !this.playback.live &&
      restart(this)
    ) {
      this.restarted = true;
      return;
    }
    interrupted?.(this);
  }
}

/** The server ends a session two minutes after it last heard of it. */
const sessionLifetime = 100_000;
/** Beats every 10 s to keep sessions alive, and knows when they lapsed. */
export class Heartbeat {
  private heardAt = Date.now();
  private timer: ReturnType<typeof setInterval>;
  constructor(beat: () => void) {
    this.timer = setInterval(beat, 10_000);
  }
  /** The server heard of the sessions just now. */
  heard() {
    this.heardAt = Date.now();
  }
  /** Count the sessions as lapsed from now on. */
  lapse() {
    this.heardAt = 0;
  }
  /** Nothing reached the server for longer than it keeps a session. */
  get lapsed() {
    return Date.now() - this.heardAt >= sessionLifetime;
  }
  stop() {
    clearInterval(this.timer);
  }
}
