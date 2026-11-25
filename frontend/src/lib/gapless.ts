import { api } from './api';
import { captureSession } from './session';
import type { Queue, QueueChange, Repeat } from './music/queue';
import {
  capabilities,
  mediaUrl,
  playbackRequest,
  replayGain,
  scheduleBuffer,
  type MediaChoice,
  type MediaInfo,
  type Playback,
} from './playback';
import {
  Heartbeat,
  PlaybackSession,
  type SessionSetup,
} from './playback-session';
/** A track streamed through a media element instead of decoded in memory. */
type Stream = {
  element: HTMLAudioElement;
  source: MediaElementAudioSourceNode;
  gain: GainNode;
  started: boolean;
  ended: boolean;
};
type Entry = {
  choice: MediaChoice;
  /** The choice's place in the Queue. */
  at: number;
  session: PlaybackSession;
  buffer?: AudioBuffer;
  stream?: Stream;
  start: number;
  offset: number;
  end: number;
  node?: ReturnType<typeof scheduleBuffer>;
};
export type MusicState = {
  /** The current track, or '' once the queue has finished. */
  id: string;
  title: string;
  position: number;
  duration: number;
  paused: boolean;
  index: number;
  count: number;
  shuffle: boolean;
  repeat: Repeat;
  /** The queue in play order; a new array whenever it changes. */
  queue: MediaChoice[];
  finished: boolean;
  /** The track at `index` is still being read. */
  loading: boolean;
  /** The streamed track at `index` waits for more of its file. */
  buffering: boolean;
  /** The track at `index` left its playlist, and goes as it stops. */
  leaving: boolean;
};
const memoryCodecs = [
  'flac',
  'pcm_s16le',
  'pcm_s24le',
  'pcm_f32le',
  'pcm_s32le',
];
/**
 * Seconds a download may take. A track due now waits briefly for the first
 * byte; its rate is judged after `sampleNow`, or `sampleAhead` for one due later,
 * whose rate has time to settle. It may finish this `rest` past when it is due,
 * and never `overdue` past it, as a rate says nothing of a file's end stalling.
 */
const downloadPatience = {
  firstByte: 1.5,
  sampleNow: 0.5,
  sampleAhead: 3,
  rest: 1.5,
  overdue: 3.5,
};
/** A track due to start is decoded this many seconds before it plays. */
const decodeMargin = 3;
/** A stream reads its own file until this many seconds from its end; then the next track loads. */
const loadAhead = 60;
/** How many seconds from a stream's end the track after it goes on the audio clock. */
const followStream = 1.5;
const android = /Android/.test(navigator.userAgent);
let silence: string | undefined;
/**
 * Android lists a page's playback from its audio elements alone, not from Web
 * Audio. This one loops silence for as long as nothing else stands for the
 * queue; ten seconds is long enough for Android to treat it as music.
 */
function silentElement() {
  if (!silence) {
    const rate = 8000;
    const samples = rate * 10;
    const header = new DataView(new ArrayBuffer(44));
    const tag = (at: number, value: string) =>
      [...value].forEach((c, i) => header.setUint8(at + i, c.charCodeAt(0)));
    tag(0, 'RIFF');
    header.setUint32(4, 36 + samples, true);
    tag(8, 'WAVEfmt ');
    header.setUint32(16, 16, true);
    header.setUint16(20, 1, true);
    header.setUint16(22, 1, true);
    header.setUint32(24, rate, true);
    header.setUint32(28, rate, true);
    header.setUint16(32, 1, true);
    header.setUint16(34, 8, true);
    tag(36, 'data');
    header.setUint32(40, samples, true);
    silence = URL.createObjectURL(
      new Blob([header, new Uint8Array(samples).fill(128)], {
        type: 'audio/wav',
      }),
    );
  }
  const element = new Audio(silence);
  element.loop = true;
  return element;
}
/**
 * Plays a Queue. Tracks that fit in memory and arrive in time are decoded and
 * scheduled back to back on one audio clock, so albums play without gaps. Other
 * tracks stream through an audio element, which starts on a few seconds of file.
 */
export class GaplessAudio {
  private context = new AudioContext();
  private volume = this.context.createGain();
  /** The playing track and the one loaded after it. */
  private entries: Entry[] = [];
  /** Bumped when the entries ahead are dropped, so a load in flight is discarded. */
  private generation = 0;
  private closed = false;
  private closing?: Promise<void>;
  private ownsSession = captureSession();
  private loading = false;
  private initialized = false;
  private timer: ReturnType<typeof setInterval>;
  private heartbeat: Heartbeat;
  private lastCurrent?: Entry;
  private preparation = new AbortController();
  private presence = android ? silentElement() : undefined;
  /** The element the system pauses and resumes for the queue while it plays. */
  private sounding?: HTMLAudioElement;
  /** The queue plays unless paused, by the user or the system. */
  private playing = true;
  /** Where the next track read starts, after a session ended under the queue. */
  private resumeAt = 0;
  /** The session of the track being read, which has no entry to keep it alive. */
  private reading?: PlaybackSession;
  /** The queue's side of each track's playback session. */
  private sessions: SessionSetup = {
    owns: this.ownsSession,
    loadHls: async () => (await import('hls.js')).default,
    hooks: {
      interrupted: (session) => {
        if (this.closed || !this.entries.some((e) => e.session === session))
          return;
        this.failed(new Error('Playback was interrupted.'));
        void this.close();
      },
      refused: (_, e) => this.refused(e),
    },
  };
  /** Opened as it was left: nothing is read until it plays. */
  private parked = false;
  constructor(
    private queue: Queue,
    private changed: (s: MusicState) => void,
    private failed: (e: unknown) => void,
    private waiting?: (value: boolean) => void,
  ) {
    this.volume.connect(this.context.destination);
    this.timer = setInterval(() => this.update(), 100);
    this.heartbeat = new Heartbeat(() => void this.keepalive());
    if (this.presence) this.followSystem(this.presence);
    queue.attach({
      holding: () => ({ held: this.entries.length, reading: this.loading }),
      changed: (change) => this.realign(change),
    });
    if (queue.restored) {
      this.parked = true;
      this.playing = false;
      this.resumeAt = queue.current?.position ?? 0;
    }
  }
  async start() {
    if (!this.parked) await this.load();
    // The second track loads while the first plays.
    if (this.closed) return;
    this.initialized = true;
    this.schedule(this.context.currentTime + 0.15);
    this.update();
    // A streamed first track starts once the page may play sound.
    this.context.onstatechange = () => {
      if (this.context.state !== 'running') return;
      this.schedule(this.context.currentTime + 0.02);
      this.update();
    };
    if (!this.parked) await this.context.resume();
  }
  private async load() {
    if (this.closed || !this.queue.unread) return;
    const generation = this.generation;
    const stale = () =>
      this.closed || generation !== this.generation || !this.ownsSession();
    const first = !this.initialized && !this.entries.length;
    const { at, choice } = this.queue.read()!;
    const resumeAt = this.resumeAt;
    this.resumeAt = 0;
    const pending = { signal: this.preparation.signal, waiting: this.waiting };
    const info = await playbackRequest(
      () => api<MediaInfo>(`/catalog/${choice.id}/playback`),
      pending,
    );
    if (stale()) return;
    const source = info.sources.find(
      (s) => !choice.fileId || s.id === choice.fileId,
    );
    const memory =
      source &&
      ['auto', 'original'].includes(info.preferences.quality) &&
      source.size <= 64 * 1024 * 1024 &&
      source.duration <= 600 &&
      source.tracks
        .filter((t) => t.kind === 'audio')
        .every((t) => memoryCodecs.includes(t.codec));
    const session = await PlaybackSession.open(
      {
        choice,
        options: {
          quality: memory ? 'original' : info.preferences.quality,
          audio: null,
          subtitle: 'off',
          capabilities: capabilities(source?.probe),
        },
        at: first ? undefined : resumeAt,
        preparation: pending,
      },
      this.sessions,
    );
    const { playback } = session;
    let entry: Entry | undefined;
    this.reading = session;
    try {
      if (stale()) return;
      const buffer =
        memory && playback.mode === 'direct'
          ? await this.decode(playback, source.size, stale)
          : undefined;
      if (stale()) return;
      entry = {
        choice,
        at,
        session,
        buffer,
        start: 0,
        offset: playback.position,
        end: 0,
      };
      if (!buffer) {
        entry.stream = this.element();
        try {
          await this.attach(entry, playback.position);
        } catch (e) {
          this.release(entry);
          entry = undefined;
          throw e;
        }
        if (stale()) {
          this.release(entry);
          entry = undefined;
          return;
        }
      }
      this.entries.push(entry);
    } finally {
      this.reading = undefined;
      if (!entry) await session.discard();
    }
  }
  /**
   * The whole file, or nothing once the rate it arrives at shows it would not
   * make it before the track is due: the track then streams instead.
   */
  private async download(playback: Playback, size: number) {
    const download = new AbortController();
    const abort = () => download.abort();
    this.preparation.signal.addEventListener('abort', abort, { once: true });
    const started = performance.now();
    let flowing = started;
    let dueSince = started;
    let received = 0;
    let late = false;
    const check = setInterval(() => {
      const { firstByte, sampleNow, sampleAhead, rest, overdue } =
        downloadPatience;
      const now = performance.now();
      const head = this.entries[0];
      const due = head
        ? Math.max(0, this.duration(head) - this.position(head) - decodeMargin)
        : 0;
      if (due) dueSince = now;
      const waited = (now - flowing) / 1000;
      if ((now - dueSince) / 1000 < overdue) {
        if (received) {
          if (waited < (due ? sampleAhead : sampleNow)) return;
          if (((size - received) * waited) / received <= due + rest) return;
        } else if (due || waited < firstByte) return;
      }
      late = true;
      download.abort();
    }, 100);
    try {
      const response = await fetch(mediaUrl(playback.url), {
        signal: download.signal,
      });
      if (!response.ok) throw new Error('Unable to load music');
      const reader = response.body!.getReader();
      const chunks: Uint8Array[] = [];
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return await new Blob(chunks as BlobPart[]).arrayBuffer();
        if (!received) flowing = performance.now();
        chunks.push(value);
        received += value.length;
      }
    } catch (e) {
      if (late) return undefined;
      throw e;
    } finally {
      clearInterval(check);
      this.preparation.signal.removeEventListener('abort', abort);
    }
  }
  /** The track decoded in memory, or nothing when it should stream instead. */
  private async decode(playback: Playback, size: number, stale: () => boolean) {
    const bytes = await this.download(playback, size);
    if (!bytes || stale() || bytes.byteLength > 64 * 1024 * 1024)
      return undefined;
    const buffer = await this.context.decodeAudioData(bytes);
    return buffer.length * buffer.numberOfChannels * 4 > 128 * 1024 * 1024
      ? undefined
      : buffer;
  }
  private element(): Stream {
    const element = new Audio();
    // The desktop app reads streams from the server's origin.
    element.crossOrigin = 'anonymous';
    element.preload = 'auto';
    const source = this.context.createMediaElementSource(element);
    const gain = this.context.createGain();
    source.connect(gain).connect(this.volume);
    const stream: Stream = {
      element,
      source,
      gain,
      started: false,
      ended: false,
    };
    element.onended = () => {
      stream.ended = true;
      this.update();
    };
    element.onerror = () => {
      if (this.closed || !this.entries.some((e) => e.stream === stream)) return;
      this.failed(new Error('Playback was interrupted.'));
      void this.close();
    };
    this.followSystem(element);
    return stream;
  }
  /**
   * A call or another app's music pauses this page's elements but not its Web
   * Audio, and resumes them after. The whole queue follows.
   */
  private followSystem(element: HTMLAudioElement) {
    element.onpause = () => {
      if (this.sounding === element && !element.ended) void this.pause();
    };
    element.onplay = () => {
      if (!this.playing) void this.resume();
    };
  }
  /** Keep the session on the stream, or else on silence, as the queue plays. */
  private sound() {
    const head = this.entries[0];
    const finished = !head && !this.queue.current;
    const element =
      !this.playing || finished
        ? undefined
        : head?.stream?.started
          ? head.stream.element
          : this.presence;
    const previous = this.sounding;
    if (element === previous) return;
    this.sounding = element;
    const { presence } = this;
    if (!presence) return;
    if (element === presence) void presence.play().catch(() => {});
    // A session whose last element is gone ends, and its controls with it. One
    // paused on silence stays for as long as the player is open.
    else if (finished && previous)
      void presence
        .play()
        .catch(() => {})
        .finally(() => presence.pause());
    else if (previous === presence) presence.pause();
  }
  private attach(entry: Entry, at: number) {
    const stream = entry.stream!;
    const { playback } = entry.session;
    stream.gain.gain.value = replayGain(playback.probe, playback.replay_gain);
    return entry.session.attach(stream.element, at);
  }
  private release(entry: Entry) {
    entry.node?.source.stop();
    entry.node?.volume.disconnect();
    entry.node = undefined;
    const stream = entry.stream;
    if (!stream) return;
    stream.element.onended = null;
    stream.element.onerror = null;
    stream.element.onpause = null;
    stream.element.onplay = null;
    stream.element.pause();
    entry.session.detach();
    stream.element.removeAttribute('src');
    stream.element.load();
    stream.source.disconnect();
    stream.gain.disconnect();
  }
  private duration(entry: Entry) {
    return entry.buffer?.duration ?? entry.session.playback.duration;
  }
  private position(entry: Entry) {
    const value = entry.stream
      ? entry.session.timeline(entry.stream.element.currentTime)
      : this.context.currentTime - entry.start + entry.offset;
    return Math.max(0, Math.min(this.duration(entry), value));
  }
  private done(entry: Entry) {
    return entry.stream
      ? entry.stream.ended
      : !!entry.node && this.context.currentTime >= entry.end;
  }
  /** The stream holds all it has left to play, so nothing can stall it now. */
  private buffered({ buffered, currentTime, duration }: HTMLAudioElement) {
    for (let i = 0; i < buffered.length; i++)
      if (buffered.start(i) <= currentTime && buffered.end(i) >= duration - 0.1)
        return true;
    return false;
  }
  /**
   * Schedule decoded tracks back to back from `when`. A streamed track ends
   * the chain until its end is near and in memory, which tells when it comes.
   * Otherwise, or if the next one streams too, that one starts once it has ended.
   */
  private schedule(when: number) {
    for (const [i, entry] of this.entries.entries()) {
      if (entry.stream) {
        if (i > 0) return;
        const { element } = entry.stream;
        if (
          !entry.stream.started &&
          this.playing &&
          this.context.state === 'running'
        ) {
          entry.stream.started = true;
          void element.play().catch(() => {});
        }
        const left = element.duration - element.currentTime;
        if (!(left < followStream) || element.paused || !this.buffered(element))
          return;
        when = this.context.currentTime + left;
        continue;
      }
      if (entry.node) {
        when = entry.end;
        continue;
      }
      entry.start = when;
      entry.node = scheduleBuffer(
        this.context,
        entry.buffer!,
        when,
        entry.offset,
        replayGain(
          entry.session.playback.probe,
          entry.session.playback.replay_gain,
        ),
        this.volume,
      );
      // Timers slow down on a hidden page; this still catches the end on time.
      entry.node.source.onended = () => this.update();
      entry.end = entry.node.end;
      when = entry.end;
    }
  }
  private snapshot(current: Entry | null): MusicState {
    // While a track loads, the state already names it.
    const choice = current?.choice ?? this.queue.current;
    return {
      id: choice?.id ?? '',
      title: choice?.title ?? '',
      position: current
        ? this.position(current)
        : this.parked
          ? this.resumeAt
          : 0,
      duration: current ? this.duration(current) : 0,
      paused: !current || this.context.state !== 'running',
      index: this.queue.index,
      count: this.queue.length,
      shuffle: this.queue.shuffled,
      repeat: this.queue.repeat,
      queue: this.queue.choices,
      finished: !choice,
      loading: !current && !!choice && !this.parked,
      buffering:
        !!current?.stream?.started &&
        !current.stream.ended &&
        this.context.state === 'running' &&
        current.stream.element.readyState < HTMLMediaElement.HAVE_FUTURE_DATA,
      leaving: this.queue.leaving,
    };
  }
  private update() {
    if (this.closed || !this.initialized) return;
    let at = this.queue.index;
    while (this.entries.length && this.done(this.entries[0])) {
      const finished = this.entries.shift()!;
      void finished.session.stop(this.duration(finished));
      this.release(finished);
      this.lastCurrent = undefined;
      at = this.entries[0]?.at ?? Math.min(finished.at + 1, this.queue.length);
      if (this.entries[0] && !this.entries[0].node)
        this.schedule(this.context.currentTime + 0.02);
    }
    if (this.entries.length) at = this.entries[0].at;
    if (this.queue.reached(at)) return;
    if (this.entries[0]?.stream && this.entries[1] && !this.entries[1].node)
      this.schedule(0);
    const head = this.entries[0];
    if (
      !this.parked &&
      this.entries.length < 2 &&
      !this.loading &&
      this.queue.unread &&
      (!head?.stream || this.duration(head) - this.position(head) <= loadAhead)
    ) {
      this.loading = true;
      void this.load()
        .then(() => {
          if (this.closed || !this.ownsSession()) return;
          this.schedule(
            Math.max(
              this.context.currentTime + 0.05,
              this.entries[0]?.end ?? 0,
            ),
          );
        })
        .catch((e) => {
          if (this.closed || !this.ownsSession()) return;
          this.failed(e);
          void this.close();
        })
        .finally(() => {
          this.loading = false;
          this.update();
        });
    }
    const current = this.entries[0] ?? null;
    if (current) {
      const position = this.position(current);
      const playing =
        this.context.state === 'running' &&
        (current.stream
          ? current.stream.started && !current.stream.element.paused
          : this.context.currentTime >= current.start);
      const activityChanged = current.session.activity.setActive(playing);
      if (this.lastCurrent !== current || activityChanged) {
        this.lastCurrent = current;
        void current.session.report(playing ? 'playing' : 'paused', position);
      }
    }
    this.sound();
    this.changed(this.snapshot(current));
  }
  /**
   * The server refused a session. Under a paused queue, or after it heard
   * nothing for too long, that ended it and no more: the track is read again.
   */
  private refused(e: unknown) {
    if (this.closed) return;
    if (!this.playing) this.heartbeat.lapse();
    else if (this.wake()) this.update();
    else {
      this.failed(e);
      void this.close();
    }
  }
  /**
   * A page frozen in the background misses its heartbeats and the server ends
   * its sessions. The track is read again from where it stopped.
   */
  private wake() {
    if (!this.heartbeat.lapsed) return false;
    this.heartbeat.heard();
    this.reading = undefined;
    const current = this.entries[0];
    if (!current) return false;
    this.resumeAt = this.position(current);
    this.discard(this.entries);
    this.entries = [];
    this.lastCurrent = undefined;
    this.queue.readFrom(this.queue.index);
    this.generation++;
    return true;
  }
  private async keepalive() {
    if (this.closed || !this.ownsSession()) return;
    // Playing on reads the track again; a report now would be refused.
    if (!this.playing && this.heartbeat.lapsed) return;
    for (const e of this.entries) {
      if (this.closed || !this.ownsSession()) return;
      if (!(await e.session.keepalive())) return;
    }
    const { reading } = this;
    if (reading && !this.closed && this.ownsSession())
      if (!(await reading.keepalive())) return;
    this.heartbeat.heard();
    const current = this.entries[0];
    if (current)
      await current.session.report(
        this.context.state === 'running' ? 'playing' : 'paused',
        this.position(current),
      );
  }
  toggle() {
    return !this.parked && this.context.state === 'running'
      ? this.pause()
      : this.resume();
  }
  async pause() {
    this.playing = false;
    this.sounding = undefined;
    this.presence?.pause();
    this.entries[0]?.stream?.element.pause();
    await this.context.suspend();
    this.update();
  }
  /** Play on; a finished queue plays again from its start. */
  async resume() {
    this.parked = false;
    this.wake();
    if (!this.entries.length && !this.queue.current) return this.jump(0);
    this.playing = true;
    this.sounding = undefined;
    await this.context.resume();
    const stream = this.entries[0]?.stream;
    if (stream?.started) await stream.element.play().catch(() => {});
    else this.schedule(this.context.currentTime + 0.02);
    this.update();
  }
  setVolume(value: number) {
    this.volume.gain.value = Math.max(0, Math.min(1, value));
  }
  async seek(position: number) {
    if (this.parked || this.wake()) {
      this.resumeAt = Math.max(0, position);
      this.update();
      return;
    }
    const current = this.entries[0];
    if (!current) return;
    current.session.activity.setActive(false);
    const to = Math.min(Math.max(0, position), this.duration(current) - 0.001);
    for (const e of this.entries) {
      if (e.stream) continue;
      e.node?.source.stop();
      e.node?.volume.disconnect();
      e.node = undefined;
      e.offset = 0;
    }
    if (current.stream) {
      current.stream.ended = false;
      const direct = current.session.playback.mode === 'direct';
      // A transcode starts again where the track is sought.
      const sought = await current.session.seek(to, {
        current: () => !this.closed && this.entries[0] === current,
      });
      if (!direct) {
        if (!sought) return;
        if (this.context.state === 'running')
          await current.stream.element.play().catch(() => {});
      }
    } else current.offset = to;
    this.schedule(this.context.currentTime + 0.02);
    this.update();
  }
  /** Silence the playing track at once and report it stopped. */
  private stopCurrent() {
    const current = this.entries[0];
    if (!current) return;
    const position = this.position(current);
    current.session.activity.setActive(false);
    this.release(current);
    void current.session.stop(position);
  }
  /** Silence entries and end their sessions. */
  private discard(entries: Entry[]) {
    for (const e of entries) {
      this.release(e);
      void e.session.discard();
    }
  }
  /** Follow a change of the Queue. */
  private realign(change: QueueChange) {
    if (this.closed) return;
    if (change === 'playing')
      return this.queue.current
        ? void this.jump(this.queue.index)
        : this.finish();
    const [current, ...ahead] = this.entries;
    if (change === 'ahead') {
      // Read again under the new order.
      this.discard(ahead);
      this.entries = current ? [current] : [];
      if (current) current.at = this.queue.index;
      this.generation++;
    } else if (change === 'moved') {
      let at = this.queue.index;
      for (const entry of this.entries) {
        entry.at = at;
        at = this.queue.after(at);
      }
    }
    this.update();
  }
  /** Stop: nothing is left to play. */
  private finish() {
    this.parked = false;
    this.stopCurrent();
    this.discard(this.entries.slice(1));
    this.entries = [];
    this.generation++;
    this.update();
  }
  /** Play the choice at `at` from its start. */
  async jump(at: number) {
    if (this.closed || at < 0 || at >= this.queue.length) return;
    this.parked = false;
    this.wake();
    this.resumeAt = 0;
    this.playing = true;
    this.stopCurrent();
    this.discard(this.entries.slice(1));
    this.entries = [];
    this.lastCurrent = undefined;
    this.queue.readFrom(at);
    this.generation++;
    // Playing a track resumes a paused queue.
    await this.context.resume();
    this.update();
  }
  /** Play the next track; repeating one track does not hold this back. */
  async skip() {
    if (!this.wake() && !this.entries.length && !this.parked) return;
    this.parked = false;
    this.resumeAt = 0;
    const target = this.queue.nextTrack();
    if (target < this.queue.length)
      return this.entries[1]?.at === target
        ? this.jumpAhead()
        : this.jump(target);
    this.queue.readFrom(this.queue.length);
    this.finish();
  }
  /** Move to what is loaded next, keeping it instead of reading it again. */
  private async jumpAhead() {
    const current = this.entries[0]!;
    this.playing = true;
    this.stopCurrent();
    this.entries.shift();
    this.lastCurrent = undefined;
    if (this.entries.length) this.queue.moveTo(this.entries[0].at);
    else {
      this.queue.readFrom(this.queue.after(current.at));
      this.generation++;
    }
    await this.context.resume();
    for (const e of this.entries) {
      if (e.stream) continue;
      e.node?.source.stop();
      e.node?.volume.disconnect();
      e.node = undefined;
    }
    const first = this.entries[0];
    if (first && !first.stream) first.offset = 0;
    this.schedule(this.context.currentTime + 0.02);
    this.update();
  }
  /** Restart the track, or a few seconds in, play the one before it. */
  async previous() {
    const current = this.entries[0];
    const position = current ? this.position(current) : this.resumeAt;
    const earlier = this.queue.previous(position);
    if (current && earlier === null) {
      await this.seek(0);
      return;
    }
    await this.jump(earlier ?? this.queue.index);
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true;
    this.preparation.abort();
    this.queue.close();
    clearInterval(this.timer);
    this.heartbeat.stop();
    this.sounding = undefined;
    if (this.presence) {
      this.presence.onpause = null;
      this.presence.onplay = null;
      this.presence.pause();
      this.presence.removeAttribute('src');
      this.presence.load();
    }
    const entries = this.entries;
    this.entries = [];
    const positions = entries.map((e) => this.position(e));
    const playing = entries.map((e) => !!e.node || !!e.stream?.started);
    for (const e of entries) {
      e.session.activity.setActive(false);
      this.release(e);
    }
    // The audio is freed once the context closes; a queue replacing this one
    // waits for that, not for the server to answer the reports.
    void (async () => {
      for (const [i, e] of entries.entries()) {
        if (!this.ownsSession()) break;
        if (i === 0 && playing[i]) await e.session.stop(positions[i]);
        else await e.session.discard();
      }
    })();
    this.closing = this.context.close();
    return this.closing;
  }
}
