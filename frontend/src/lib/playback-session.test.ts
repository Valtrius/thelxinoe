import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type Hls from 'hls.js';
import { ApiError } from './api';
import type { Playback } from './playback';
import {
  Heartbeat,
  PlaybackSession,
  type ProgressReport,
  type SeekAnswer,
  type SessionApi,
  type SessionHooks,
  type SessionSetup,
} from './playback-session';

type Call =
  | {
      kind: 'progress';
      id: string;
      report: ProgressReport;
      signal?: AbortSignal;
    }
  | { kind: 'seek'; id: string; position: number }
  | { kind: 'keepalive' | 'end'; id: string };

/** A server whose answers the test settles, one request at a time. */
function fakeServer() {
  const calls: Call[] = [];
  const pending: {
    resolve: (value?: unknown) => void;
    reject: (error: unknown) => void;
  }[] = [];
  const answer = (call: Call) => {
    calls.push(call);
    return new Promise((resolve, reject) => pending.push({ resolve, reject }));
  };
  const api: SessionApi = {
    prepare: async () => playback(),
    progress: (id, report, signal) =>
      answer({ kind: 'progress', id, report, signal }),
    seek: (id, position) =>
      answer({ kind: 'seek', id, position }) as Promise<SeekAnswer>,
    keepalive: (id) => answer({ kind: 'keepalive', id }),
    end: (id) => answer({ kind: 'end', id }),
  };
  return {
    api,
    calls,
    /** Settles the oldest unsettled request. */
    resolve: (value?: unknown) => pending.shift()!.resolve(value),
    reject: (error: unknown) => pending.shift()!.reject(error),
  };
}

class FakeHls {
  static supported = true;
  static instances: FakeHls[] = [];
  static Events = {
    MANIFEST_PARSED: 'manifest',
    FRAG_BUFFERED: 'buffered',
    ERROR: 'error',
  };
  static isSupported() {
    return FakeHls.supported;
  }
  handlers = new Map<string, ((event: string, data: unknown) => void)[]>();
  source?: string;
  media?: unknown;
  destroyed = false;
  constructor(public config: Record<string, unknown>) {
    FakeHls.instances.push(this);
  }
  loadSource(url: string) {
    this.source = url;
  }
  attachMedia(media: unknown) {
    this.media = media;
  }
  on(event: string, handler: (event: string, data: unknown) => void) {
    this.handlers.set(event, [...(this.handlers.get(event) ?? []), handler]);
  }
  emit(event: string, data: unknown = {}) {
    for (const handler of this.handlers.get(event) ?? []) handler(event, data);
  }
  destroy() {
    this.destroyed = true;
  }
}

function fakeElement({ nativeHls = false } = {}) {
  return {
    currentTime: 0,
    src: '',
    onloadedmetadata: null as (() => void) | null,
    canPlayType: () => (nativeHls ? 'maybe' : ''),
  };
}
type FakeElement = ReturnType<typeof fakeElement>;
const media = (element: FakeElement) => element as unknown as HTMLMediaElement;

function playback(overrides: Partial<Playback> = {}): Playback {
  return {
    id: 'session',
    url: '/playback/session/stream.m3u8',
    mode: 'transcode',
    position: 0,
    duration: 600,
    timeline_start: 0,
    video: true,
    tracks: [],
    subtitles: [],
    selected_subtitle: null,
    options: {} as Playback['options'],
    chapters: [],
    probe: {},
    replay_gain: 'off',
    ...overrides,
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve));

let server: ReturnType<typeof fakeServer>;
function session(
  overrides: Partial<Playback> = {},
  setup: Partial<SessionSetup> = {},
) {
  return new PlaybackSession(playback(overrides), {
    api: server.api,
    loadHls: () => FakeHls as unknown as typeof Hls,
    ...setup,
  });
}

beforeEach(() => {
  server = fakeServer();
  FakeHls.supported = true;
  FakeHls.instances = [];
  vi.stubGlobal('location', { href: 'http://localhost/' });
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('progress reports', () => {
  it('numbers reports and sends each once the one before it settled', async () => {
    const opened = session();
    void opened.report('playing', 1);
    void opened.report('paused', 2);
    await flush();
    expect(server.calls).toHaveLength(1);

    // A failed report doesn't hold back the next.
    server.reject(new Error('offline'));
    await flush();
    expect(
      server.calls.map((call) => call.kind === 'progress' && call.report),
    ).toEqual([
      { sequence: 0, position: 1, state: 'playing', active_seconds: 0 },
      { sequence: 1, position: 2, state: 'paused', active_seconds: 0 },
    ]);
  });

  it('passes a refusal to the caller and plays on through a transient failure', async () => {
    const refused = vi.fn();
    const opened = session({}, { hooks: { refused } });
    void opened.report('playing', 1);
    await flush();
    server.reject(new ApiError(503, 'unavailable', 'Unavailable'));
    await flush();
    expect(refused).not.toHaveBeenCalled();

    void opened.report('playing', 11);
    await flush();
    const error = new ApiError(404, 'not_found', 'Not found');
    server.reject(error);
    await flush();
    expect(refused).toHaveBeenCalledExactlyOnceWith(opened, error);

    void opened.keepalive().then((heard) => expect(heard).toBe(false));
    await flush();
    server.reject(error);
    await flush();
    expect(refused).toHaveBeenCalledTimes(2);
  });

  it('sends nothing once the user signed out', async () => {
    const opened = session({}, { owns: () => false });
    await opened.report('playing', 1);
    expect(await opened.keepalive()).toBe(false);
    await opened.discard();
    expect(server.calls).toEqual([]);
  });
});

describe('ending a session', () => {
  it('reports it stopped once and sends nothing after', async () => {
    const opened = session({ mode: 'direct', url: '/movie.mp4' });
    const element = fakeElement();
    await opened.attach(media(element), 0);
    const stopped = opened.stop(42);
    expect(opened.stop(50)).toBe(stopped);
    expect(element.onloadedmetadata).toBeNull();
    await flush();
    server.resolve();
    await stopped;

    await opened.report('playing', 43);
    expect(await opened.seek(10)).toBe(false);
    expect(element.currentTime).toBe(0);
    await opened.keepalive();
    const other = fakeElement();
    await opened.attach(media(other), 0);
    expect(other.src).toBe('');
    expect(server.calls).toEqual([
      {
        kind: 'progress',
        id: 'session',
        report: {
          sequence: 0,
          position: 42,
          state: 'stopped',
          active_seconds: 0,
        },
        signal: undefined,
      },
    ]);
  });

  it('cancels the pending reports of a Player session it stops', async () => {
    const opened = session({}, { abortableReports: true });
    void opened.report('playing', 1);
    void opened.report('paused', 2);
    void opened.stop(3);
    await flush();
    server.resolve();
    await flush();
    server.resolve();
    await flush();
    const [first, second, stopped] = server.calls as Extract<
      Call,
      { kind: 'progress' }
    >[];
    expect(first.signal?.aborted).toBe(true);
    expect(second.signal?.aborted).toBe(true);
    expect(stopped.report.state).toBe('stopped');
    expect(stopped.signal?.aborted).toBe(false);
  });

  it('keeps a session open after a stopped report on reaching the end', async () => {
    const opened = session();
    void opened.report('stopped', 600);
    void opened.report('paused', 600);
    await flush();
    server.resolve();
    await flush();
    expect(opened.stopped).toBe(false);
    expect(server.calls).toHaveLength(2);
  });

  it('ends a session that never played as its caller chose', async () => {
    const discarded = session({ id: 'discarded' });
    void discarded.discard();
    const abandoned = session({ id: 'abandoned', position: 12 });
    const ending = abandoned.abandon();
    await flush();
    server.resolve();
    server.reject(new Error('offline'));
    await expect(ending).rejects.toThrow('offline');
    expect(server.calls).toEqual([
      { kind: 'end', id: 'discarded' },
      {
        kind: 'progress',
        id: 'abandoned',
        report: { sequence: 0, position: 12, state: 'stopped' },
        signal: undefined,
      },
    ]);
  });
});

describe('streams', () => {
  it('restarts a transcode on the timeline a seek answers with', async () => {
    const order: string[] = [];
    const opened = session(
      {},
      {
        hooks: {
          moved: () => order.push('moved'),
          attaching: () => void order.push('attaching'),
        },
      },
    );
    const element = fakeElement();
    await opened.attach(media(element), 30);
    const [first] = FakeHls.instances;
    expect(first.config.startPosition).toBe(30);
    expect(first.media).toBe(element);

    const seeking = opened.seek(700, { autoplay: true });
    await flush();
    server.resolve({
      url: '/playback/session/from-598.m3u8',
      timeline_start: 598,
      position: 599.95,
    });
    expect(await seeking).toBe(true);
    expect(server.calls).toEqual([
      { kind: 'seek', id: 'session', position: 599.95 },
    ]);
    const [, second] = FakeHls.instances;
    expect(first.destroyed).toBe(true);
    expect(second.source).toBe('/playback/session/from-598.m3u8');
    expect(second.config.startPosition).toBeCloseTo(1.95);
    expect(opened.playback.timeline_start).toBe(598);
    expect(opened.timeline(1)).toBe(599);
    expect(opened.mediaTime(500)).toBe(0);
    expect(order).toEqual(['attaching', 'moved', 'attaching']);
  });

  it('drops a seek answer its caller moved on from', async () => {
    const opened = session();
    await opened.attach(media(fakeElement()), 0);
    const seeking = opened.seek(100, { current: () => false });
    await flush();
    server.resolve({ url: '/b.m3u8', timeline_start: 100, position: 100 });
    expect(await seeking).toBe(false);
    expect(opened.playback.timeline_start).toBe(0);
    expect(FakeHls.instances).toHaveLength(1);
  });

  it('seeks a direct stream in place', async () => {
    const opened = session({ mode: 'direct', url: '/movie.mp4' });
    const element = fakeElement();
    await opened.attach(media(element), 0);
    expect(await opened.seek(90)).toBe(true);
    expect(element.currentTime).toBe(90);
    expect(server.calls).toEqual([]);
  });

  it('starts a file stream where asked, at its start only for the Player', async () => {
    const play = vi.fn();
    const player = session(
      { mode: 'direct', url: '/movie.mp4' },
      { seekAtStart: true, hooks: { play } },
    );
    const video = fakeElement();
    video.currentTime = 7;
    await player.attach(media(video), 0, true);
    video.onloadedmetadata!();
    expect(video.currentTime).toBe(0);
    expect(play).toHaveBeenCalledOnce();

    const track = session({ mode: 'direct', url: '/track.flac' });
    const audio = fakeElement();
    await track.attach(media(audio), 0);
    expect(audio.onloadedmetadata).toBeNull();
    await track.attach(media(audio), 25);
    audio.onloadedmetadata!();
    expect(audio.currentTime).toBe(25);
  });

  it('plays a playlist natively where hls.js is unsupported', async () => {
    FakeHls.supported = false;
    const opened = session();
    const element = fakeElement({ nativeHls: true });
    await opened.attach(media(element), 0);
    expect(element.src).toBe('/playback/session/stream.m3u8');
    expect(FakeHls.instances).toEqual([]);
  });

  it('restarts a failed transcode once when its caller can', async () => {
    let ready = false;
    const hooks = {
      restart: vi.fn(() => ready),
      interrupted: vi.fn(),
    } satisfies SessionHooks;
    const opened = session({}, { hooks });
    await opened.attach(media(fakeElement()), 0);
    const [stream] = FakeHls.instances;

    stream.emit('error', { fatal: false });
    expect(hooks.interrupted).not.toHaveBeenCalled();
    // A busy caller can't restart now; the stream is interrupted.
    stream.emit('error', { fatal: true });
    expect(hooks.interrupted).toHaveBeenCalledTimes(1);
    ready = true;
    stream.emit('error', { fatal: true });
    expect(hooks.restart).toHaveBeenCalledTimes(2);
    expect(hooks.interrupted).toHaveBeenCalledTimes(1);
    stream.emit('error', { fatal: true });
    expect(hooks.restart).toHaveBeenCalledTimes(2);
    expect(hooks.interrupted).toHaveBeenCalledTimes(2);
  });

  it('closes on a failed stream without a restart hook, as the music dock does', async () => {
    const interrupted = vi.fn();
    const opened = session({}, { hooks: { interrupted } });
    await opened.attach(media(fakeElement()), 0);
    FakeHls.instances[0].emit('error', { fatal: true });
    expect(interrupted).toHaveBeenCalledExactlyOnceWith(opened);
  });

  it('ignores a stream it detached while hls.js loaded', async () => {
    let load!: (value: typeof Hls) => void;
    const opened = session(
      {},
      { loadHls: () => new Promise((resolve) => (load = resolve)) },
    );
    const attaching = opened.attach(media(fakeElement()), 0);
    opened.detach();
    load(FakeHls as unknown as typeof Hls);
    await attaching;
    expect(FakeHls.instances).toEqual([]);
  });
});

describe('heartbeat', () => {
  it('beats every 10 s and lapses when the server heard nothing for 100 s', () => {
    vi.useFakeTimers();
    const beat = vi.fn();
    const heartbeat = new Heartbeat(beat);
    vi.advanceTimersByTime(30_000);
    expect(beat).toHaveBeenCalledTimes(3);
    heartbeat.heard();
    vi.advanceTimersByTime(99_000);
    expect(heartbeat.lapsed).toBe(false);
    vi.advanceTimersByTime(1_000);
    expect(heartbeat.lapsed).toBe(true);
    heartbeat.heard();
    expect(heartbeat.lapsed).toBe(false);
    heartbeat.lapse();
    expect(heartbeat.lapsed).toBe(true);
    heartbeat.stop();
    vi.advanceTimersByTime(30_000);
    expect(beat).toHaveBeenCalledTimes(13);
  });

  it('keeps a session alive until it stops', async () => {
    const opened = session();
    const alive = opened.keepalive();
    await flush();
    server.resolve();
    expect(await alive).toBe(true);
    void opened.stop(0);
    expect(await opened.keepalive()).toBe(true);
    await flush();
    expect(server.calls.map((call) => call.kind)).toEqual([
      'keepalive',
      'progress',
    ]);
  });
});
