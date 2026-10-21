import { describe, expect, it } from 'vitest';
import type { YoutubeDownload, YoutubeVideo } from './types';
import { youtubeVideoPresentation } from './youtube-video-presentation';

const context = {
  mpvReady: true,
  ytdlpReady: true,
  ffmpegReady: true,
  isPlaying: false,
  isLaunching: false,
  downloadBusy: false,
};
function downloaded(format: Partial<YoutubeDownload>) {
  const video = {
    videoId: 'xwdamnfS6IM',
    positionSeconds: 0,
    watchedPercentage: 0,
    isWatched: false,
    broadcastState: 'none',
    availabilityStatus: 'available',
    download: {
      videoId: 'xwdamnfS6IM',
      status: 'ready',
      fileSizeBytes: 540 * 1024 ** 2,
      pinned: false,
      requestedAt: '',
      ...format,
    },
  } as YoutubeVideo;
  return youtubeVideoPresentation(video, context);
}

describe('YouTube download format', () => {
  it.each([
    [
      { widthPixels: 3840, heightPixels: 2160, frameRate: 24000 / 1001 },
      '2160p24',
    ],
    [{ widthPixels: 3840, heightPixels: 2160, frameRate: 60 }, '2160p60'],
    [
      { widthPixels: 1920, heightPixels: 1080, frameRate: 30000 / 1001 },
      '1080p30',
    ],
    // Vertical Shorts are named by their short side, as YouTube names them.
    [{ widthPixels: 1080, heightPixels: 1920, frameRate: 30 }, '1080p30'],
    [{ heightPixels: 720 }, '720p'],
  ])('names %o %s', (format, label) => {
    const presentation = downloaded(format);
    expect(presentation.downloadFormatLabel).toBe(label);
    expect(presentation.readyDownloadTitle).toBe(`540 MB local · ${label}`);
  });

  it('has no format without a probed picture', () => {
    expect(downloaded({}).downloadFormatLabel).toBeNull();
  });
});
