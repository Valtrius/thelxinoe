import { describe, expect, it } from 'vitest';
import { mediaFacts } from './media-facts';

describe('media facts', () => {
  it('summarize picture, sound and subtitles from ffprobe', () => {
    expect(
      mediaFacts({
        format: { size: '6871947674' },
        streams: [
          {
            codec_type: 'video',
            codec_name: 'mjpeg',
            disposition: { attached_pic: 1 },
          },
          {
            codec_type: 'video',
            codec_name: 'hevc',
            width: 3840,
            height: 1600,
            pix_fmt: 'yuv420p10le',
            color_transfer: 'smpte2084',
          },
          {
            codec_type: 'audio',
            codec_name: 'eac3',
            channels: 6,
            tags: { language: 'eng' },
          },
          {
            codec_type: 'audio',
            codec_name: 'aac',
            channels: 2,
            tags: { language: 'fre' },
          },
          {
            codec_type: 'audio',
            codec_name: 'aac',
            channels: 2,
            tags: { language: 'fre' },
          },
          {
            codec_type: 'subtitle',
            codec_name: 'subrip',
            tags: { language: 'eng' },
          },
          {
            codec_type: 'subtitle',
            codec_name: 'subrip',
            tags: { language: 'fre' },
          },
          {
            codec_type: 'subtitle',
            codec_name: 'ass',
            tags: { language: 'eng' },
          },
          {
            codec_type: 'subtitle',
            codec_name: 'ass',
            tags: { language: 'und' },
          },
        ],
      }),
    ).toEqual({
      video: '2160p · HEVC 10-bit · HDR10 · 6.4 GB',
      audio: 'ENG EAC3 5.1, FRE AAC 2.0',
      subtitles: 'ENG, FRE',
    });
  });

  it.each([
    [3840, 2160, '2160p'],
    [3840, 1600, '2160p'],
    [1920, 1080, '1080p'],
    [1920, 800, '1080p'],
    [1280, 720, '720p'],
    [1280, 534, '720p'],
    [854, 480, '480p'],
    [720, 576, '576p'],
    [2160, 3840, '2160p'],
    [1600, 3840, '2160p'],
    [1080, 1920, '1080p'],
    [800, 1920, '1080p'],
    [720, 1280, '720p'],
    [534, 1280, '720p'],
    [480, 854, '480p'],
    [360, 640, '360p'],
    [1080, 1080, '1080p'],
    [undefined, 1080, '1080p'],
    [1920, undefined, '1080p'],
    [640, undefined, ''],
  ])('name a %s by %s picture %s', (width, height, label) => {
    expect(
      mediaFacts({ streams: [{ codec_type: 'video', width, height }] }).video,
    ).toBe(label);
  });

  it('leave out what ffprobe did not report', () => {
    expect(mediaFacts({})).toEqual({ video: '', audio: '', subtitles: '' });
    expect(
      mediaFacts({
        streams: [
          { codec_type: 'video', codec_name: 'h264', width: 1280, height: 534 },
          { codec_type: 'audio', codec_name: 'opus', channels: 3 },
        ],
      }),
    ).toEqual({ video: '720p · H.264', audio: 'Opus 3 ch', subtitles: '' });
  });
});
