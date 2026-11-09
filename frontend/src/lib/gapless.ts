import type Hls from 'hls.js';
import { api } from './api';
import { inBackground } from './page-readiness';
import { captureSession } from './session';
import {
  appendedQueue,
  followedQueue,
  insertedQueue,
  playsAfter,
  removedQueue,
  wraps,
} from './music/queue-follow';
import { PlaybackActivity } from './playback-activity';
import {
  capabilities,
  mediaUrl,
  prepare,
  playbackRequest,
  replayGain,
  scheduleBuffer,
  sessionRefused,
  type MediaChoice,
  type MediaInfo,
  type Playback,
} from './playback';
/** A track streamed through a media element instead of decoded in memory. */
type Stream = {
  element: HTMLAudioElement;
  source: MediaElementAudioSourceNode;
  gain: GainNode;
  hls?: Hls;
  started: boolean;
  ended: boolean;
};
type Entry = {
  choice: MediaChoice;
  /** The choice's place in the queue. */
  at: number;
  playback: Playback;
  buffer?: AudioBuffer;
  stream?: Stream;
  sequence: number;
  start: number;
  offset: number;
  end: number;
  node?: ReturnType<typeof scheduleBuffer>;
  reports: Promise<unknown>;
  activity: PlaybackActivity;
};
export type Repeat = 'off' | 'all' | 'one';
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
/** The server ends a session two minutes after its last heartbeat. */
const sessionLifetime = 100_000;
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
 * A music queue. Tracks that fit in memory and arrive in time are decoded and
 * scheduled back to back on one audio clock, so albums play without gaps. Other
 * tracks stream through an audio element, which starts on a few seconds of file.
 */
export class GaplessQueue {
  private context = new AudioContext();
  private volume = this.context.createGain();
  private entries: Entry[] = [];
  private choices: MediaChoice[];
  /** The queue before it was shuffled. */
  private order: MediaChoice[];
  private shuffled = false;
  private repeat: Repeat = 'off';
  /** The playing choice, when its playlist lost it: it goes as it stops. */
  private leaving?: MediaChoice;
  /** The playing choice's place in `choices`. */
  private index: number;
  /** The next choice to load. */
  private next: number;
  /** Bumped when the entries ahead are dropped, so a load in flight is discarded. */
  private generation = 0;
  private closed = false;
  private closing?: Promise<void>;
  private ownsSession = captureSession();
  private loading = false;
  private initialized = false;
  private timer: ReturnType<typeof setInterval>;
  private heartbeat: ReturnType<typeof setInterval>;
  private lastCurrent?: Entry;
  private preparation = new AbortController();
  private presence = android ? silentElement() : undefined;
  /** The element the system pauses and resumes for the queue while it plays. */
  private sounding?: HTMLAudioElement;
  /** The queue plays unless paused, by the user or the system. */
  private playing = true;
  private lastHeartbeat = Date.now();
  /** Where the next track read starts, after a session ended under the queue. */
  private resumeAt = 0;
  /** The session of the track being read, which has no entry to keep it alive. */
  private reading?: string;
  /** Opened as it was left: nothing is read until it plays. */
  private parked = false;
  /** Reading the rest of the list this queue started from; clearing ends it. */
  private extension = new AbortController();
  /** Tracks added by hand, which the rest of the list doesn't add again. */
  private inserted = new Set<string>();
  constructor(
    choices: MediaChoice[],
    private changed: (s: MusicState) => void,
    private failed: (e: unknown) => void,
    private waiting?: (value: boolean) => void,
    start = 0,
    parked?: MediaChoice['parked'],
  ) {
    this.choices = [...choices];
    this.order = [...choices];
    this.index = Math.min(Math.max(0, start), Math.max(0, choices.length - 1));
    this.next = this.index;
    this.volume.connect(this.context.destination);
    this.timer = setInterval(() => this.update(), 100);
    this.heartbeat = setInterval(() => void this.keepalive(), 10000);
    if (this.presence) this.followSystem(this.presence);
    if (parked) {
      this.parked = true;
      this.playing = false;
      this.repeat = parked.repeat;
      this.shuffled = parked.shuffled;
      if (parked.leaving) this.leaving = this.choices[this.index];
      this.resumeAt = this.choices[this.index]?.position ?? 0;
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
  private get upcoming() {
    return {
      choices: this.choices,
      repeat: this.repeat,
      leaving: this.leaving,
    };
  }
  /** What plays after the choice at `at`. */
  private advance(at: number) {
    return playsAfter(this.upcoming, at);
  }
  private forgetLeaving() {
    const at = this.choices.indexOf(this.leaving!);
    this.choices = this.choices.filter((choice) => choice !== this.leaving);
    this.order = this.order.filter((choice) => choice !== this.leaving);
    this.leaving = undefined;
    if (at >= 0 && at < this.index) this.index--;
    this.resetAhead();
  }
  private async load() {
    if (this.closed || this.next >= this.choices.length) return;
    const generation = this.generation;
    const stale = () =>
      this.closed || generation !== this.generation || !this.ownsSession();
    const first = !this.initialized && !this.entries.length;
    const at = this.next;
    const choice = this.choices[at];
    this.next = this.advance(at);
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
    const playback = await prepare(
      choice,
      {
        quality: memory ? 'original' : info.preferences.quality,
        audio: null,
        subtitle: 'off',
        capabilities: capabilities(source?.probe),
      },
      first ? undefined : resumeAt,
      pending,
    );
    let entry: Entry | undefined;
    this.reading = playback.id;
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
        playback,
        buffer,
        sequence: 0,
        start: 0,
        offset: playback.position,
        end: 0,
        reports: Promise.resolve(),
        activity: new PlaybackActivity(),
      };
      if (!buffer) {
        entry.stream = this.element();
        try {
          await this.attach(entry, playback.url, playback.position);
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
      if (!entry && this.ownsSession())
        await api(`/playback/${playback.id}`, 'DELETE').catch(() => {});
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
    const finished = !head && !this.choices[this.index];
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
  private async attach(entry: Entry, url: string, at: number) {
    const stream = entry.stream!;
    const { element } = stream;
    stream.gain.gain.value = replayGain(
      entry.playback.probe,
      entry.playback.replay_gain,
    );
    stream.hls?.destroy();
    stream.hls = undefined;
    const from = Math.max(0, at - entry.playback.timeline_start);
    const playlist = new URL(url, location.href).pathname.endsWith('.m3u8');
    const { default: HlsPlayer } = playlist
      ? await import('hls.js')
      : { default: undefined };
    if (
      !HlsPlayer ||
      (!HlsPlayer.isSupported() &&
        element.canPlayType('application/vnd.apple.mpegurl'))
    ) {
      element.src = mediaUrl(url);
      element.onloadedmetadata = from
        ? () => (element.currentTime = from)
        : null;
      return;
    }
    const hls = new HlsPlayer({
      enableWorker: false,
      startPosition: from,
      maxBufferLength: 30,
    });
    stream.hls = hls;
    hls.loadSource(mediaUrl(url));
    hls.attachMedia(element);
    hls.on(HlsPlayer.Events.ERROR, (_, data) => {
      if (!data.fatal || this.closed || !this.entries.includes(entry)) return;
      this.failed(new Error('Playback was interrupted.'));
      void this.close();
    });
  }
  private release(entry: Entry) {
    entry.node?.source.stop();
    entry.node?.volume.disconnect();
    entry.node = undefined;
    const stream = entry.stream;
    if (!stream) return;
    stream.element.onended = null;
    stream.element.onloadedmetadata = null;
    stream.element.onerror = null;
    stream.element.onpause = null;
    stream.element.onplay = null;
    stream.element.pause();
    stream.hls?.destroy();
    stream.hls = undefined;
    stream.element.removeAttribute('src');
    stream.element.load();
    stream.source.disconnect();
    stream.gain.disconnect();
  }
  private duration(entry: Entry) {
    return entry.buffer?.duration ?? entry.playback.duration;
  }
  private position(entry: Entry) {
    const value = entry.stream
      ? entry.stream.element.currentTime + entry.playback.timeline_start
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
        replayGain(entry.playback.probe, entry.playback.replay_gain),
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
    const choice = current?.choice ?? this.choices[this.index];
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
      index: this.index,
      count: this.choices.length,
      shuffle: this.shuffled,
      repeat: this.repeat,
      queue: this.choices,
      finished: !choice,
      loading: !current && !!choice && !this.parked,
      buffering:
        !!current?.stream?.started &&
        !current.stream.ended &&
        this.context.state === 'running' &&
        current.stream.element.readyState < HTMLMediaElement.HAVE_FUTURE_DATA,
      leaving: !!this.leaving,
    };
  }
  private update() {
    if (this.closed || !this.initialized) return;
    while (this.entries.length && this.done(this.entries[0])) {
      const finished = this.entries.shift()!;
      finished.activity.setActive(false);
      void this.report(finished, 'stopped', this.duration(finished));
      this.release(finished);
      this.lastCurrent = undefined;
      this.index =
        this.entries[0]?.at ?? Math.min(finished.at + 1, this.choices.length);
      if (this.entries[0] && !this.entries[0].node)
        this.schedule(this.context.currentTime + 0.02);
    }
    if (this.entries.length) this.index = this.entries[0].at;
    if (this.leaving && this.choices[this.index] !== this.leaving)
      return this.forgetLeaving();
    if (this.entries[0]?.stream && this.entries[1] && !this.entries[1].node)
      this.schedule(0);
    const head = this.entries[0];
    if (
      !this.parked &&
      this.entries.length < 2 &&
      !this.loading &&
      this.next < this.choices.length &&
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
      const activityChanged = current.activity.setActive(playing);
      if (this.lastCurrent !== current || activityChanged) {
        this.lastCurrent = current;
        void this.report(current, playing ? 'playing' : 'paused', position);
      }
    }
    this.sound();
    this.changed(this.snapshot(current));
  }
  private report(entry: Entry, state: string, position: number) {
    const sequence = entry.sequence++;
    const activeSeconds = entry.activity.seconds();
    entry.reports = entry.reports
      .catch(() => {})
      // Progress reports never hold back the page being opened.
      .then(() =>
        this.ownsSession()
          ? inBackground(() =>
              api(`/playback/${entry.playback.id}/progress`, 'POST', {
                sequence,
                state,
                position,
                active_seconds: activeSeconds,
              }),
            )
          : undefined,
      );
    return entry.reports.catch((e) => {
      if (sessionRefused(e)) this.refused(e);
    });
  }
  private lapsed() {
    return Date.now() - this.lastHeartbeat >= sessionLifetime;
  }
  /**
   * The server refused a session. Under a paused queue, or after it heard
   * nothing for too long, that ended it and no more: the track is read again.
   */
  private refused(e: unknown) {
    if (this.closed) return;
    if (!this.playing) this.lastHeartbeat = 0;
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
    if (!this.lapsed()) return false;
    this.lastHeartbeat = Date.now();
    this.reading = undefined;
    const current = this.entries[0];
    if (!current) return false;
    this.resumeAt = this.position(current);
    this.discard(this.entries);
    this.entries = [];
    this.lastCurrent = undefined;
    this.next = this.index;
    this.generation++;
    return true;
  }
  private async keepalive() {
    if (this.closed || !this.ownsSession()) return;
    // Playing on reads the track again; a report now would be refused.
    if (!this.playing && this.lapsed()) return;
    try {
      for (const e of this.entries) {
        if (this.closed || !this.ownsSession()) return;
        await inBackground(() =>
          api(`/playback/${e.playback.id}/keepalive`, 'POST'),
        );
      }
      const { reading } = this;
      if (reading && !this.closed && this.ownsSession())
        await inBackground(() => api(`/playback/${reading}/keepalive`, 'POST'));
      this.lastHeartbeat = Date.now();
      const current = this.entries[0];
      if (current)
        await this.report(
          current,
          this.context.state === 'running' ? 'playing' : 'paused',
          this.position(current),
        );
    } catch (e) {
      if (sessionRefused(e)) this.refused(e);
    }
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
    if (!this.entries.length && !this.choices[this.index]) return this.jump(0);
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
    current.activity.setActive(false);
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
      if (current.playback.mode === 'direct')
        current.stream.element.currentTime = to;
      else {
        // A conversion starts again where the track is sought.
        const next = await api<{
          url: string;
          timeline_start: number;
          position: number;
        }>(`/playback/${current.playback.id}/seek`, 'POST', {
          position: Math.min(to, current.playback.duration - 0.05),
        });
        if (this.closed || this.entries[0] !== current) return;
        current.playback = { ...current.playback, ...next };
        await this.attach(current, next.url, next.position);
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
    current.activity.setActive(false);
    this.release(current);
    void this.report(current, 'stopped', position);
  }
  /** Silence entries and end their sessions. */
  private discard(entries: Entry[]) {
    for (const e of entries) {
      this.release(e);
      if (this.ownsSession())
        void api(`/playback/${e.playback.id}`, 'DELETE').catch(() => {});
    }
  }
  /** Drop what was loaded ahead, so it is read again under the new order. */
  private resetAhead() {
    const [current, ...ahead] = this.entries;
    this.discard(ahead);
    this.entries = current ? [current] : [];
    if (current) current.at = this.index;
    this.next = current
      ? this.advance(this.index)
      : Math.min(this.index, this.choices.length);
    this.generation++;
    this.update();
  }
  /** Play the choice at `at` from its start. */
  async jump(at: number) {
    if (this.closed || at < 0 || at >= this.choices.length) return;
    this.parked = false;
    this.wake();
    this.resumeAt = 0;
    this.playing = true;
    this.stopCurrent();
    this.discard(this.entries.slice(1));
    this.entries = [];
    this.lastCurrent = undefined;
    this.index = at;
    this.next = at;
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
    const target =
      this.index + 1 < this.choices.length
        ? this.index + 1
        : wraps(this.upcoming)
          ? 0
          : this.choices.length;
    if (target < this.choices.length)
      return this.entries[1]?.at === target
        ? this.jumpAhead()
        : this.jump(target);
    this.stopCurrent();
    this.discard(this.entries.slice(1));
    this.entries = [];
    this.index = this.choices.length;
    this.next = this.choices.length;
    this.generation++;
    this.update();
  }
  /** Move to what is loaded next, keeping it instead of reading it again. */
  private async jumpAhead() {
    const current = this.entries[0]!;
    this.playing = true;
    this.stopCurrent();
    this.entries.shift();
    this.lastCurrent = undefined;
    this.index = this.entries[0]?.at ?? this.advance(current.at);
    if (!this.entries.length) {
      this.next = this.index;
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
    const earlier =
      this.index > 0
        ? this.index - 1
        : this.repeat === 'all'
          ? this.choices.length - 1
          : -1;
    if (current && (position > 3 || earlier < 0)) {
      await this.seek(0);
      return;
    }
    await this.jump(earlier < 0 || position > 3 ? this.index : earlier);
  }
  /** Add tracks after the playing one, or at the end. */
  insert(items: MediaChoice[], next: boolean) {
    if (this.closed || !items.length) return;
    for (const item of items) this.inserted.add(item.id);
    const finished = !this.entries.length && this.next >= this.choices.length;
    const { choices, order, at } = insertedQueue(
      {
        choices: this.choices,
        order: this.order,
        index: this.index,
        shuffled: this.shuffled,
        finished,
      },
      items,
      next,
    );
    this.choices = choices;
    this.order = order;
    if (finished) {
      this.index = at;
      this.next = at;
      this.generation++;
      this.update();
    } else this.resetAhead();
  }
  async remove(at: number) {
    if (this.closed || at < 0 || at >= this.choices.length) return;
    const { choices, order, index } = removedQueue(
      { choices: this.choices, order: this.order, index: this.index },
      at,
    );
    this.choices = choices;
    this.order = order;
    if (at === this.index) {
      if (at < this.choices.length) return this.jump(at);
      this.parked = false;
      this.stopCurrent();
      this.discard(this.entries.slice(1));
      this.entries = [];
      this.index = this.choices.length;
      this.next = this.choices.length;
      this.generation++;
      this.update();
      return;
    }
    this.index = index;
    this.resetAhead();
  }
  /**
   * Take the order of the playlist this queue follows. What is already loaded
   * ahead keeps playing when it still comes next.
   */
  follow(items: MediaChoice[]) {
    if (this.closed || this.index >= this.choices.length) return;
    const upNext = this.choices[this.advance(this.index)];
    const followed = followedQueue(
      {
        choices: this.choices,
        index: this.index,
        shuffled: this.shuffled,
      },
      items,
    );
    this.choices = followed.choices;
    this.order = followed.order;
    this.index = followed.index;
    const playing = this.choices[this.index];
    this.leaving = items.some((item) => item.id === playing.id)
      ? undefined
      : playing;
    const [current, ahead] = this.entries;
    if (
      this.loading ||
      !current ||
      upNext !== this.choices[this.advance(this.index)]
    ) {
      this.resetAhead();
      return;
    }
    current.at = this.index;
    this.next = this.advance(this.index);
    if (ahead) {
      ahead.at = this.next;
      this.next = this.advance(ahead.at);
    }
    this.update();
  }
  /**
   * Read what comes after this queue's tracks and add it when it arrives. A
   * queue that can't get it plays on with what it has.
   */
  extend(read: NonNullable<MediaChoice['more']>) {
    const { signal } = this.extension;
    if (signal.aborted) return;
    read(signal).then(
      (items) => {
        if (!signal.aborted) this.append(items);
      },
      () => {},
    );
  }
  private append(list: MediaChoice[]) {
    if (this.closed || !this.ownsSession()) return;
    const appended = appendedQueue(
      {
        ...this.upcoming,
        order: this.order,
        index: this.index,
        next: this.next,
        shuffled: this.shuffled,
        loaded: this.loading || this.entries.length > 0,
      },
      list,
      this.inserted,
    );
    if (!appended) return;
    this.choices = appended.choices;
    this.order = appended.order;
    if (appended.reset) return this.resetAhead();
    this.next = appended.next;
    this.update();
  }
  /** Keep only the playing track. */
  clear() {
    if (this.closed) return;
    this.extension.abort();
    const current = this.choices[this.index];
    this.choices = current ? [current] : [];
    this.order = [...this.choices];
    this.index = 0;
    this.leaving = undefined;
    this.resetAhead();
  }
  setShuffle(on: boolean) {
    if (this.closed || on === this.shuffled) return;
    const current = this.choices[this.index];
    if (on) {
      this.order = [...this.choices];
      const rest = this.choices.filter((_, i) => i !== this.index);
      for (let i = rest.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [rest[i], rest[j]] = [rest[j], rest[i]];
      }
      this.choices = current ? [current, ...rest] : rest;
      this.index = 0;
    } else {
      this.choices = [...this.order];
      this.index = Math.max(0, this.choices.indexOf(current));
    }
    this.shuffled = on;
    this.resetAhead();
  }
  setRepeat(mode: Repeat) {
    if (this.closed || mode === this.repeat) return;
    this.repeat = mode;
    this.resetAhead();
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true;
    this.preparation.abort();
    this.extension.abort();
    clearInterval(this.timer);
    clearInterval(this.heartbeat);
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
      e.activity.setActive(false);
      this.release(e);
    }
    // The audio is freed once the context closes; a queue replacing this one
    // waits for that, not for the server to answer the reports.
    void (async () => {
      for (const [i, e] of entries.entries()) {
        if (!this.ownsSession()) break;
        if (i === 0 && playing[i])
          await this.report(e, 'stopped', positions[i]);
        else await api(`/playback/${e.playback.id}`, 'DELETE').catch(() => {});
      }
    })();
    this.closing = this.context.close();
    return this.closing;
  }
}
