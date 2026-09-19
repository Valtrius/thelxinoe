import Hls from 'hls.js';
import { api, serverUrl } from './api';

export type MediaChoice = {
  kind?: string;
  id: string;
  title: string;
  fileId?: string;
  restore?: boolean;
  position?: number;
  queueIndex?: number;
  queue_context?: { client_id: string; revision: number; index: number };
  queue?: MediaChoice[];
};
export type Preferences = {
  quality: string;
  audio_language: string;
  subtitle_language: string;
  subtitles: boolean;
  replay_gain: string;
};
export type Track = {
  id: string;
  index: number | null;
  kind: string;
  codec: string;
  language: string;
  title: string;
  supported: boolean;
};
export type Options = {
  quality: string;
  audio: number | null;
  subtitle: string | null;
  capabilities: ReturnType<typeof capabilities>;
};
export type Playback = {
  id: string;
  url: string;
  mode: string;
  position: number;
  duration: number;
  timeline_start: number;
  video: boolean;
  tracks: Track[];
  subtitles: { id: string; language: string; title: string; url: string }[];
  selected_subtitle: string | null;
  options: Options;
  probe: Probe;
  replay_gain: string;
};
export type Probe = {
  format?: { tags?: Record<string, string> };
  streams?: { tags?: Record<string, string> }[];
};
export type MediaInfo = {
  sources: {
    id: string;
    edition: string;
    duration: number;
    video: boolean;
    tracks: Track[];
    probe: Probe;
    size: number;
  }[];
  preferences: Preferences;
  progress: { edition: string; position: number; duration: number }[];
  watched: boolean;
};
export function capabilities() {
  const media = document.createElement('video');
  const supports = (type: string) => !!media.canPlayType(type);
  return {
    containers: [
      'mp4',
      'm4v',
      'm4a',
      'mp3',
      'wav',
      'flac',
      'ogg',
      'opus',
      'webm',
    ],
    video: [
      ...(supports('video/mp4; codecs="avc1.42E01E"') ? ['h264'] : []),
      ...(supports('video/webm; codecs="vp9"') ? ['vp9'] : []),
      ...(supports('video/webm; codecs="vp8"') ? ['vp8'] : []),
      ...(supports('video/mp4; codecs="av01.0.04M.08"') ? ['av1'] : []),
    ],
    audio: [
      'aac',
      'mp3',
      'flac',
      'vorbis',
      'opus',
      'pcm_s16le',
      'pcm_s24le',
      'pcm_f32le',
    ],
    hls: Hls.isSupported() || supports('application/vnd.apple.mpegurl'),
  };
}
export function mediaUrl(path: string) {
  return `${serverUrl()}${path}`;
}
export function prepare(
  choice: MediaChoice,
  options: Options,
  position?: number,
) {
  return api<Playback>('/playback', 'POST', {
    media_id: choice.id,
    file_id: choice.fileId,
    options,
    position: position ?? choice.position,
    queue: choice.queue_context,
  });
}
export function replayGain(probe: Probe, mode: string): number {
  if (mode === 'off') return 1;
  const tags = Object.fromEntries(
    [
      ...Object.entries(probe.format?.tags ?? {}),
      ...(probe.streams ?? []).flatMap((s) => Object.entries(s.tags ?? {})),
    ].map(([k, v]) => [k.toUpperCase(), v]),
  );
  const key =
    mode === 'album' && tags.REPLAYGAIN_ALBUM_GAIN ? 'ALBUM' : 'TRACK';
  const db = parseFloat(tags[`REPLAYGAIN_${key}_GAIN`] ?? '0');
  const peak = parseFloat(tags[`REPLAYGAIN_${key}_PEAK`] ?? '0');
  const gain =
    10 ** (Math.max(-30, Math.min(20, Number.isFinite(db) ? db : 0)) / 20);
  return Number.isFinite(peak) && peak > 0 ? Math.min(gain, 1 / peak) : gain;
}
export function scheduleBuffer(
  context: BaseAudioContext,
  buffer: AudioBuffer,
  when: number,
  offset: number,
  gain: number,
  destination: AudioNode = context.destination,
) {
  const source = context.createBufferSource();
  source.buffer = buffer;
  const volume = context.createGain();
  volume.gain.value = gain;
  source.connect(volume).connect(destination);
  source.start(when, offset);
  return { source, volume, end: when + buffer.duration - offset };
}
export function time(value: number) {
  const seconds = Math.max(0, Math.floor(value));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
