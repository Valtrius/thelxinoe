import { formatBytes } from '../providers/utils';

type Stream = {
  codec_type?: string;
  codec_name?: string;
  width?: number;
  height?: number;
  pix_fmt?: string;
  color_transfer?: string;
  channels?: number;
  tags?: { language?: string };
  disposition?: { attached_pic?: number };
  side_data_list?: { side_data_type?: string }[];
};
/** What ffprobe reported for a library file. */
export type Probe = {
  format?: { duration?: string; size?: string };
  streams?: Stream[];
};

const codecs: Record<string, string> = {
  hevc: 'HEVC',
  h264: 'H.264',
  av1: 'AV1',
  vp9: 'VP9',
  mpeg2video: 'MPEG-2',
  eac3: 'EAC3',
  ac3: 'AC3',
  aac: 'AAC',
  truehd: 'TrueHD',
  dts: 'DTS',
  flac: 'FLAC',
  opus: 'Opus',
  mp3: 'MP3',
};
const codec = (stream: Stream) =>
  stream.codec_name
    ? (codecs[stream.codec_name] ?? stream.codec_name.toUpperCase())
    : '';
const layouts: Record<number, string> = {
  1: 'Mono',
  2: '2.0',
  6: '5.1',
  8: '7.1',
};
const language = (stream: Stream) => {
  const value = stream.tags?.language;
  return value && value !== 'und' ? value.toUpperCase() : '';
};
// Widescreen films are cropped in height, so the width decides first.
function resolution({ width = 0, height = 0 }: Stream) {
  if (width >= 3800 || height >= 2000) return '2160p';
  if (width >= 1900 || height >= 1000) return '1080p';
  if (width >= 1260 || height >= 700) return '720p';
  return height ? `${height}p` : '';
}
function range(stream: Stream) {
  if (stream.side_data_list?.some((d) => d.side_data_type?.includes('DOVI')))
    return 'Dolby Vision';
  if (stream.color_transfer === 'smpte2084') return 'HDR10';
  if (stream.color_transfer === 'arib-std-b67') return 'HLG';
  return '';
}
const line = (parts: string[], separator: string) =>
  [...new Set(parts.filter(Boolean))].join(separator);

/** A file's picture, sound and subtitles in short lines; empty when unknown. */
export function mediaFacts(probe: Probe) {
  const streams = probe.streams ?? [];
  const video = streams.find(
    (s) => s.codec_type === 'video' && s.disposition?.attached_pic !== 1,
  );
  const size = Number(probe.format?.size);
  return {
    video: line(
      [
        video ? resolution(video) : '',
        video
          ? `${codec(video)}${video.pix_fmt?.includes('10') ? ' 10-bit' : ''}`
          : '',
        video ? range(video) : '',
        size > 0 ? formatBytes(size) : '',
      ],
      ' · ',
    ),
    audio: line(
      streams
        .filter((s) => s.codec_type === 'audio')
        .map((s) =>
          [
            language(s),
            codec(s),
            s.channels ? (layouts[s.channels] ?? `${s.channels} ch`) : '',
          ]
            .filter(Boolean)
            .join(' '),
        ),
      ', ',
    ),
    subtitles: line(
      streams.filter((s) => s.codec_type === 'subtitle').map(language),
      ', ',
    ),
  };
}
