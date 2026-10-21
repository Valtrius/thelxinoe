import Hls from 'hls.js';
import { api, ApiError, serverUrl } from './api';
import { captureSession } from './session';

export type MediaChoice = {
  kind?: string;
  id: string;
  title: string;
  fileId?: string;
  restore?: boolean;
  /** The desktop MPV run a restored player follows. */
  run?: string;
  resolveYoutube?: boolean;
  position?: number;
  /** This choice's place in `queue`, or in the queue it was taken from. */
  queueIndex?: number;
  queue?: MediaChoice[];
};
/** Media that is playing, or still starting in a desktop MPV window. */
export type PlayingMedia = { id: string; launching: boolean };
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
  file_id?: string;
  generation?: string;
  live?: boolean;
  id: string;
  url: string;
  mode: string;
  encoding?: { video: boolean; audio: boolean };
  conversion_reason?: string | null;
  qualities?: { value: string; label: string }[];
  quality?: string;
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
  streams?: {
    tags?: Record<string, string>;
    codec_type?: string;
    codec_name?: string;
    profile?: string;
    level?: number;
    pix_fmt?: string;
    height?: number;
    avg_frame_rate?: string;
    r_frame_rate?: string;
  }[];
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
export function capabilities(probe: Probe = {}) {
  const media = document.createElement('video');
  const supports = (type: string) => !!media.canPlayType(type);
  const nativeHls = supports('application/vnd.apple.mpegurl');
  const mseHls = Hls.isSupported();
  const source =
    globalThis.MediaSource ??
    (
      globalThis as typeof globalThis & {
        ManagedMediaSource?: typeof MediaSource;
      }
    ).ManagedMediaSource;
  const hlsSupports = (type: string) =>
    mseHls
      ? (source?.isTypeSupported(type) ?? false)
      : nativeHls && supports(type);
  const candidates: Record<string, string[]> = {
    h264: [66, 77, 100, 110, 122, 244].flatMap((profile) =>
      [30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62].flatMap((level) =>
        (profile === 66 ? [0, 224] : [0]).map(
          (flags) =>
            `avc1.${[profile, flags, level].map((v) => v.toString(16).padStart(2, '0').toUpperCase()).join('')}`,
        ),
      ),
    ),
    hevc: [1, 2].flatMap((profile) =>
      [90, 93, 120, 123, 150, 153, 156, 180, 183, 186].map(
        (level) => `hvc1.${profile}.6.L${level}.B0`,
      ),
    ),
    vp8: ['vp8'],
    vp9: ['vp09.00.10.08', 'vp09.02.10.10'],
    // A browser can decode AV1 only up to a level; YouTube's 4K60 is 13.
    av1: ['08', '10'].flatMap((depth) =>
      Array.from(
        { length: 20 },
        (_, level) => `av01.0.${String(level).padStart(2, '0')}M.${depth}`,
      ),
    ),
    aac: ['mp4a.40.2', 'mp4a.40.5', 'mp4a.40.29', 'mp4a.40.4'],
    mp3: ['mp4a.6B'],
    flac: ['flac'],
    vorbis: ['vorbis'],
    opus: ['opus'],
    ac3: ['ac-3'],
    eac3: ['ec-3'],
  };
  for (const stream of probe.streams ?? []) {
    if (stream.codec_name === 'h264' && stream.level) {
      const profile =
        (
          {
            'Constrained Baseline': 66,
            Baseline: 66,
            Main: 77,
            'High 10': 110,
            'High 4:2:2': 122,
            'High 4:4:4 Predictive': 244,
          } as Record<string, number>
        )[stream.profile ?? ''] ?? 100;
      const flags = stream.profile === 'Constrained Baseline' ? 224 : 0;
      candidates.h264.push(
        `avc1.${[profile, flags, stream.level].map((v) => v.toString(16).padStart(2, '0').toUpperCase()).join('')}`,
      );
    }
  }
  const supported_media: string[] = supports('audio/wav') ? ['audio/wav'] : [];
  const hls_media: string[] = [];
  const video: string[] = [],
    audio: string[] = [],
    hls_video: string[] = [],
    hls_audio: string[] = [];
  for (const [codec, container] of [
    ['flac', 'flac'],
    ['mp3', 'mpeg'],
  ]) {
    if (supports(`audio/${container}`)) {
      audio.push(codec);
      supported_media.push(`audio/${container}`);
    }
  }
  for (const [codec, strings] of Object.entries(candidates)) {
    const isVideo = ['h264', 'hevc', 'vp8', 'vp9', 'av1'].includes(codec);
    for (const name of strings) {
      for (const container of ['mp4', 'webm', 'ogg', 'mpeg', 'flac']) {
        const type = `${isVideo ? 'video' : 'audio'}/${container}; codecs="${name}"`;
        if (supports(type)) {
          supported_media.push(type);
          (isVideo ? video : audio).push(codec);
        }
        if (container === 'mp4' && hlsSupports(type)) {
          hls_media.push(type);
          (isVideo ? hls_video : hls_audio).push(codec);
          (isVideo ? video : audio).push(codec);
        }
      }
    }
  }
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
    video: [...new Set(video)],
    audio: [
      ...new Set([
        ...audio,
        ...(supports('audio/wav')
          ? ['pcm_s16le', 'pcm_s24le', 'pcm_f32le']
          : []),
      ]),
    ],
    hls_video: [...new Set(hls_video)],
    hls_audio: [...new Set(hls_audio)],
    supported_media,
    hls_media,
    hls: mseHls || nativeHls,
  };
}
export function mediaUrl(path: string) {
  return `${serverUrl()}${path}`;
}
// The server answers for a session it no longer plays with a 4xx. A dropped
// connection or a server error is transient: playback goes on and the next
// report catches up.
export function sessionRefused(error: unknown) {
  return error instanceof ApiError && error.status < 500;
}
export type Preparation = {
  signal: AbortSignal;
  waiting?: (value: boolean) => void;
};
export async function playbackRequest<T>(
  request: () => Promise<T>,
  preparation: Preparation,
): Promise<T> {
  const { signal, waiting } = preparation;
  const ownsSession = captureSession();
  try {
    for (;;) {
      signal.throwIfAborted();
      if (!ownsSession())
        throw new DOMException('Playback canceled', 'AbortError');
      try {
        return await request();
      } catch (error) {
        if (!(error instanceof ApiError) || error.code !== 'tools_preparing')
          throw error;
        signal.throwIfAborted();
        waiting?.(true);
        await new Promise<void>((resolve, reject) => {
          const cancel = () => {
            clearTimeout(timer);
            reject(signal.reason);
          };
          const timer = setTimeout(() => {
            signal.removeEventListener('abort', cancel);
            resolve();
          }, 1000);
          signal.addEventListener('abort', cancel, { once: true });
        });
      }
    }
  } finally {
    waiting?.(false);
  }
}
export function prepare(
  choice: MediaChoice,
  options: Options,
  position?: number,
  preparation?: Preparation,
) {
  const body = {
    media_id: choice.id,
    file_id: choice.fileId,
    options,
    position: position ?? choice.position,
  };
  const request = () => api<Playback>('/playback', 'POST', body);
  return preparation ? playbackRequest(request, preparation) : request();
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

export function qualityOptions(
  playback: Playback | null,
  info: MediaInfo | null,
  quality: string,
  delivered = playback?.quality,
) {
  const selected = quality === 'auto' ? delivered : undefined;
  const automatic = {
    value: 'auto',
    label: selected ? `Auto (${selected})` : 'Auto',
  };
  if (playback?.qualities?.length) return [automatic, ...playback.qualities];
  if (playback && !playback.video)
    return [automatic, { value: 'original', label: 'Original' }];
  const height = (
    info?.sources
      .flatMap((source) => source.probe.streams ?? [])
      .find((stream) => stream.codec_type === 'video') ??
    playback?.probe.streams?.find((stream) => stream.codec_type === 'video')
  )?.height;
  return [
    automatic,
    { value: 'original', label: `Original${height ? ` (${height}p)` : ''}` },
    ...[2160, 1440, 1080, 720, 480, 360]
      .filter((value) => value <= (height ?? 1080))
      .map((value) => ({ value: `${value}p`, label: `${value}p` })),
  ];
}
