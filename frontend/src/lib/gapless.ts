import type Hls from 'hls.js';
import { api } from './api';
import { inBackground } from './page-readiness';
import { captureSession } from './session';
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
};
const memoryCodecs = [
  'flac',
  'pcm_s16le',
  'pcm_s24le',
  'pcm_f32le',
  'pcm_s32le',
];
/**
 * A music queue. Tracks that fit in memory are decoded and scheduled back to
 * back on one audio clock, so albums play without gaps. Other tracks stream
 * through an audio element and start as the one before them ends.
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
  constructor(
    choices: MediaChoice[],
    private changed: (s: MusicState) => void,
    private failed: (e: unknown) => void,
    private waiting?: (value: boolean) => void,
    start = 0,
  ) {
    this.choices = [...choices];
    this.order = [...choices];
    this.index = Math.min(Math.max(0, start), Math.max(0, choices.length - 1));
    this.next = this.index;
    this.volume.connect(this.context.destination);
    this.timer = setInterval(() => this.update(), 100);
    this.heartbeat = setInterval(() => void this.keepalive(), 10000);
  }
  async start() {
    await this.load();
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
    await this.context.resume();
  }
  /** What plays after the choice at `at`. */
  private advance(at: number) {
    if (this.repeat === 'one') return at;
    if (at + 1 < this.choices.length) return at + 1;
    return this.repeat === 'all' ? 0 : this.choices.length;
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
      first ? undefined : 0,
      pending,
    );
    let entry: Entry | undefined;
    try {
      if (stale()) return;
      const buffer =
        memory && playback.mode === 'direct'
          ? await this.decode(playback, stale)
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
      if (!entry && this.ownsSession())
        await api(`/playback/${playback.id}`, 'DELETE').catch(() => {});
    }
  }
  /** The track decoded in memory, or nothing when it is too large for that. */
  private async decode(playback: Playback, stale: () => boolean) {
    const response = await fetch(mediaUrl(playback.url), {
      signal: this.preparation.signal,
    });
    if (!response.ok) throw new Error('Unable to load music');
    const bytes = await response.arrayBuffer();
    if (stale() || bytes.byteLength > 64 * 1024 * 1024) return undefined;
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
    return stream;
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
      element.onloadedmetadata = () => (element.currentTime = from);
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
  /**
   * Schedule decoded tracks back to back from `when`. A streamed track ends
   * the chain: what follows it starts once it has ended.
   */
  private schedule(when: number) {
    for (const [i, entry] of this.entries.entries()) {
      if (entry.stream) {
        if (i > 0) return;
        if (!entry.stream.started && this.context.state === 'running') {
          entry.stream.started = true;
          void entry.stream.element.play().catch(() => {});
        }
        return;
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
      position: current ? this.position(current) : 0,
      duration: current ? this.duration(current) : 0,
      paused: !current || this.context.state !== 'running',
      index: this.index,
      count: this.choices.length,
      shuffle: this.shuffled,
      repeat: this.repeat,
      queue: this.choices,
      finished: !choice,
      loading: !current && !!choice,
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
    if (
      this.entries.length < 2 &&
      !this.loading &&
      this.next < this.choices.length
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
      if (!this.closed && sessionRefused(e)) {
        this.failed(e);
        void this.close();
      }
    });
  }
  private async keepalive() {
    if (this.closed || !this.ownsSession()) return;
    try {
      for (const e of this.entries) {
        if (this.closed || !this.ownsSession()) return;
        await inBackground(() =>
          api(`/playback/${e.playback.id}/keepalive`, 'POST'),
        );
      }
      const current = this.entries[0];
      if (current)
        await this.report(
          current,
          this.context.state === 'running' ? 'playing' : 'paused',
          this.position(current),
        );
    } catch (e) {
      if (!this.closed && sessionRefused(e)) {
        this.failed(e);
        await this.close();
      }
    }
  }
  async toggle() {
    const stream = this.entries[0]?.stream;
    if (this.context.state === 'running') {
      stream?.element.pause();
      await this.context.suspend();
    } else {
      await this.context.resume();
      if (stream?.started) await stream.element.play().catch(() => {});
      else this.schedule(this.context.currentTime + 0.02);
    }
    this.update();
  }
  setVolume(value: number) {
    this.volume.gain.value = Math.max(0, Math.min(1, value));
  }
  async seek(position: number) {
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
    if (!this.entries.length) return;
    const target =
      this.index + 1 < this.choices.length
        ? this.index + 1
        : this.repeat === 'off'
          ? this.choices.length
          : 0;
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
    const position = current ? this.position(current) : 0;
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
    await this.jump(earlier < 0 ? this.index : earlier);
  }
  /** Add tracks after the playing one, or at the end. */
  insert(items: MediaChoice[], next: boolean) {
    if (this.closed || !items.length) return;
    const finished = !this.entries.length && this.next >= this.choices.length;
    const at =
      next && !finished
        ? Math.min(this.index + 1, this.choices.length)
        : this.choices.length;
    this.choices = [
      ...this.choices.slice(0, at),
      ...items,
      ...this.choices.slice(at),
    ];
    this.order = this.shuffled ? [...this.order, ...items] : [...this.choices];
    if (finished) {
      this.index = at;
      this.next = at;
      this.generation++;
      this.update();
    } else this.resetAhead();
  }
  async remove(at: number) {
    if (this.closed || at < 0 || at >= this.choices.length) return;
    const removed = this.choices[at];
    this.choices = this.choices.filter((_, i) => i !== at);
    this.order = this.order.filter((choice) => choice !== removed);
    if (at === this.index) {
      if (at < this.choices.length) return this.jump(at);
      this.stopCurrent();
      this.discard(this.entries.slice(1));
      this.entries = [];
      this.index = this.choices.length;
      this.next = this.choices.length;
      this.generation++;
      this.update();
      return;
    }
    if (at < this.index) this.index--;
    this.resetAhead();
  }
  /** Keep only the playing track. */
  clear() {
    if (this.closed) return;
    const current = this.choices[this.index];
    this.choices = current ? [current] : [];
    this.order = [...this.choices];
    this.index = 0;
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
    clearInterval(this.timer);
    clearInterval(this.heartbeat);
    const entries = this.entries;
    this.entries = [];
    const positions = entries.map((e) => this.position(e));
    const playing = entries.map((e) => !!e.node || !!e.stream?.started);
    for (const e of entries) {
      e.activity.setActive(false);
      this.release(e);
    }
    this.closing = (async () => {
      await this.context.close();
      for (const [i, e] of entries.entries()) {
        if (!this.ownsSession()) break;
        if (i === 0 && playing[i])
          await this.report(e, 'stopped', positions[i]);
        else await api(`/playback/${e.playback.id}`, 'DELETE').catch(() => {});
      }
    })();
    return this.closing;
  }
}
