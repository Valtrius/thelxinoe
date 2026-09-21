import { api } from './api';
import { PlaybackActivity } from './playback-activity';
import {
  capabilities,
  mediaUrl,
  prepare,
  replayGain,
  scheduleBuffer,
  type MediaChoice,
  type MediaInfo,
  type Playback,
} from './playback';
type Entry = {
  choice: MediaChoice;
  playback: Playback;
  buffer: AudioBuffer;
  sequence: number;
  start: number;
  offset: number;
  end: number;
  node?: ReturnType<typeof scheduleBuffer>;
  reports: Promise<unknown>;
  activity: PlaybackActivity;
};
export type MusicState = {
  title: string;
  position: number;
  duration: number;
  paused: boolean;
  index: number;
  count: number;
  gain: number;
};
export class StreamingRequired extends Error {
  constructor(
    message: string,
    public choice?: MediaChoice,
  ) {
    super(message);
  }
}
export class GaplessQueue {
  private context = new AudioContext();
  private volume = this.context.createGain();
  private entries: Entry[] = [];
  private index = 0;
  private next = 0;
  private closed = false;
  private loading = false;
  private initialized = false;
  private deferred?: StreamingRequired;
  private timer: ReturnType<typeof setInterval>;
  private heartbeat: ReturnType<typeof setInterval>;
  private lastCurrent?: Entry;
  constructor(
    private choices: MediaChoice[],
    private changed: (s: MusicState) => void,
    private failed: (e: unknown) => void,
  ) {
    this.volume.connect(this.context.destination);
    this.timer = setInterval(() => this.update(), 100);
    this.heartbeat = setInterval(() => void this.keepalive(), 10000);
  }
  async start() {
    await this.load();
    try {
      await this.load();
    } catch (e) {
      if (e instanceof StreamingRequired) {
        this.deferred = e;
        this.next = this.choices.length;
      } else throw e;
    }
    if (this.closed) return;
    this.initialized = true;
    this.schedule(this.context.currentTime + 0.15);
    this.update();
    await this.context.resume();
  }
  private async load() {
    if (this.closed || this.next >= this.choices.length) return;
    const first = this.next === 0;
    const choice = this.choices[this.next++];
    const info = await api<MediaInfo>(`/catalog/${choice.id}/playback`);
    const source = info.sources.find(
      (s) => !choice.fileId || s.id === choice.fileId,
    );
    const supported =
      source &&
      ['auto', 'original'].includes(info.preferences.quality) &&
      source.size <= 64 * 1024 * 1024 &&
      source.duration <= 600 &&
      source.tracks
        .filter((t) => t.kind === 'audio')
        .every((t) =>
          ['flac', 'pcm_s16le', 'pcm_s24le', 'pcm_f32le', 'pcm_s32le'].includes(
            t.codec,
          ),
        );
    if (!supported)
      throw new StreamingRequired(
        'This track uses streaming playback.',
        choice,
      );
    if (this.closed) return;
    const playback = await prepare(
      choice,
      {
        quality: 'original',
        audio: null,
        subtitle: 'off',
        capabilities: capabilities(),
      },
      first ? undefined : 0,
    );
    try {
      if (this.closed) return;
      if (playback.mode !== 'direct')
        throw new StreamingRequired('This track requires conversion.', choice);
      const response = await fetch(mediaUrl(playback.url));
      if (!response.ok) throw new Error('Unable to load music');
      const bytes = await response.arrayBuffer();
      if (bytes.byteLength > 64 * 1024 * 1024)
        throw new StreamingRequired(
          'Track exceeds the memory playback limit.',
          choice,
        );
      const buffer = await this.context.decodeAudioData(bytes);
      if (buffer.length * buffer.numberOfChannels * 4 > 128 * 1024 * 1024)
        throw new StreamingRequired(
          'Decoded track exceeds the memory playback limit.',
          choice,
        );
      if (this.closed) return;
      this.entries.push({
        choice,
        playback,
        buffer,
        sequence: 0,
        start: 0,
        offset: playback.position,
        end: 0,
        reports: Promise.resolve(),
        activity: new PlaybackActivity(),
      });
      return;
    } finally {
      if (!this.entries.some((e) => e.playback.id === playback.id))
        await api(`/playback/${playback.id}`, 'DELETE').catch(() => {});
    }
  }
  private schedule(when: number) {
    for (const entry of this.entries) {
      if (entry.node) {
        when = entry.end;
        continue;
      }
      entry.start = when;
      entry.node = scheduleBuffer(
        this.context,
        entry.buffer,
        when,
        entry.offset,
        replayGain(entry.playback.probe, entry.playback.replay_gain),
        this.volume,
      );
      entry.end = entry.node.end;
      when = entry.end;
    }
  }
  private update() {
    if (this.closed || !this.initialized) return;
    while (
      this.entries.length &&
      this.entries[0].node &&
      this.context.currentTime >= this.entries[0].end
    ) {
      const done = this.entries.shift()!;
      done.activity.setActive(false);
      void this.report(done, 'stopped', done.buffer.duration);
      done.node?.volume.disconnect();
      this.index++;
      this.lastCurrent = undefined;
    }
    if (
      this.entries.length < 2 &&
      !this.loading &&
      this.next < this.choices.length
    ) {
      this.loading = true;
      void this.load()
        .then(() => {
          this.schedule(
            Math.max(
              this.context.currentTime + 0.05,
              this.entries[0]?.end ?? 0,
            ),
          );
        })
        .catch((e) => {
          if (e instanceof StreamingRequired) {
            this.deferred = e;
            this.next = this.choices.length;
            return;
          }
          this.failed(e);
          void this.close();
        })
        .finally(() => (this.loading = false));
    }
    const current = this.entries[0];
    if (!current) {
      if (this.deferred && !this.loading) {
        this.failed(this.deferred);
        this.deferred = undefined;
        void this.close();
        return;
      }
      if (!this.loading && this.next >= this.choices.length)
        this.changed({
          title: 'Queue finished',
          position: 0,
          duration: 0,
          paused: true,
          index: this.index,
          count: this.choices.length,
          gain: 1,
        });
      return;
    }
    const position = Math.max(
      0,
      Math.min(
        current.buffer.duration,
        this.context.currentTime - current.start + current.offset,
      ),
    );
    const playing =
      this.context.state === 'running' &&
      this.context.currentTime >= current.start;
    const activityChanged = current.activity.setActive(playing);
    if (this.lastCurrent !== current || activityChanged) {
      this.lastCurrent = current;
      void this.report(current, playing ? 'playing' : 'paused', position);
    }
    this.changed({
      title: current.choice.title,
      position,
      duration: current.buffer.duration,
      paused: this.context.state !== 'running',
      index: this.index,
      count: this.choices.length,
      gain: replayGain(current.playback.probe, current.playback.replay_gain),
    });
  }
  private report(entry: Entry, state: string, position: number) {
    const sequence = entry.sequence++;
    const activeSeconds = entry.activity.seconds();
    entry.reports = entry.reports
      .catch(() => {})
      .then(() =>
        api(`/playback/${entry.playback.id}/progress`, 'POST', {
          sequence,
          state,
          position,
          active_seconds: activeSeconds,
        }),
      );
    return entry.reports.catch((e) => {
      if (!this.closed) {
        this.failed(e);
        void this.close();
      }
    });
  }
  private async keepalive() {
    try {
      for (const e of this.entries)
        await api(`/playback/${e.playback.id}/keepalive`, 'POST');
      const current = this.entries[0];
      if (current)
        await this.report(
          current,
          this.context.state === 'running' ? 'playing' : 'paused',
          Math.max(
            0,
            this.context.currentTime - current.start + current.offset,
          ),
        );
    } catch (e) {
      if (!this.closed) {
        this.failed(e);
        await this.close();
      }
    }
  }
  async toggle() {
    if (this.context.state === 'running') await this.context.suspend();
    else await this.context.resume();
    this.update();
  }
  setVolume(value: number) {
    this.volume.gain.value = Math.max(0, Math.min(1, value));
  }
  seek(position: number) {
    const current = this.entries[0];
    if (!current) return;
    current.activity.setActive(false);
    for (const e of this.entries) {
      e.node?.source.stop();
      e.node?.volume.disconnect();
      e.node = undefined;
      e.offset = 0;
    }
    current.offset = Math.min(
      Math.max(0, position),
      current.buffer.duration - 0.001,
    );
    this.schedule(this.context.currentTime + 0.02);
    this.update();
  }
  async skip() {
    const current = this.entries[0];
    if (!current) return;
    current.activity.setActive(false);
    await this.report(
      current,
      'stopped',
      Math.max(0, this.context.currentTime - current.start + current.offset),
    );
    current.node?.source.stop();
    current.node?.volume.disconnect();
    this.entries.shift();
    this.index++;
    if (this.entries.length) {
      this.seek(0);
    }
    this.update();
  }
  async close() {
    if (this.closed) return;
    this.closed = true;
    clearInterval(this.timer);
    clearInterval(this.heartbeat);
    const position = this.context.currentTime;
    for (const [i, e] of this.entries.entries()) {
      e.activity.setActive(false);
      e.node?.source.stop();
      if (i === 0 && e.node)
        await this.report(
          e,
          'stopped',
          Math.max(0, position - e.start + e.offset),
        );
      else await api(`/playback/${e.playback.id}`, 'DELETE').catch(() => {});
    }
    this.entries = [];
    await this.context.close();
  }
}
