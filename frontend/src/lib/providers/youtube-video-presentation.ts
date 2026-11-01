import { formatBytes, formatClock } from './utils';
import type { YoutubeDownloadEvent, YoutubeVideo } from './types';

export function youtubeVideoPresentation(
  video: YoutubeVideo,
  context: {
    playerReady: boolean;
    downloadsEnabled: boolean;
    isPlaying: boolean;
    isLaunching: boolean;
    downloadBusy: boolean;
    downloadEvent?: YoutubeDownloadEvent;
  },
) {
  const {
    playerReady,
    downloadsEnabled,
    isPlaying,
    isLaunching,
    downloadBusy,
    downloadEvent,
  } = context;
  const activeDownload = downloadEvent?.download ?? video.download;
  const downloadReady = activeDownload?.status === 'ready';
  const downloadPending = [
    'queued',
    'waiting_for_space',
    'downloading',
    'processing',
  ].includes(activeDownload?.status ?? '');
  const isBusy = isPlaying || isLaunching;
  const canPlay =
    playerReady &&
    (video.availabilityStatus !== 'unavailable' || downloadReady) &&
    !video.isUpcoming &&
    !isBusy;
  const canDownload =
    downloadsEnabled &&
    video.availabilityStatus !== 'unavailable' &&
    !video.isLive &&
    !video.isUpcoming &&
    !downloadBusy;
  const tracksVideoProgress =
    video.broadcastState === 'none' || video.broadcastState === 'replay';
  const inProgress =
    tracksVideoProgress && video.positionSeconds > 1 && !video.isWatched;
  const showPosition =
    tracksVideoProgress &&
    video.positionSeconds > 1 &&
    (inProgress || isPlaying);
  const remainingSeconds = video.durationSeconds
    ? Math.max(video.durationSeconds - video.positionSeconds, 0)
    : null;
  const primaryLabel = isLaunching
    ? 'Launching'
    : isPlaying
      ? 'Already playing'
      : inProgress
        ? `Resume from ${formatClock(video.positionSeconds)}`
        : video.isLive
          ? 'Watch live'
          : video.isUpcoming
            ? 'Not live yet'
            : 'Play';
  const totalBytes = downloadEvent?.totalBytes ?? activeDownload?.totalBytes;
  const downloadedBytes =
    downloadEvent?.downloadedBytes ?? activeDownload?.downloadedBytes ?? 0;
  // Putting the streams together reports no progress of its own.
  const downloadPercent =
    totalBytes && activeDownload?.status !== 'processing'
      ? Math.min(100, (downloadedBytes / totalBytes) * 100)
      : null;
  // Formats are named like YouTube names them: by the short side, so a vertical
  // 1080×1920 Short is 1080p, followed by the frame rate, as in 2160p60.
  const resolution =
    activeDownload?.widthPixels && activeDownload.heightPixels
      ? Math.min(activeDownload.widthPixels, activeDownload.heightPixels)
      : activeDownload?.heightPixels;
  const downloadFormatLabel = resolution
    ? `${resolution}p${activeDownload?.frameRate ? Math.round(activeDownload.frameRate) : ''}`
    : null;
  let downloadLabel: string | null = null;
  if (downloadBusy && !activeDownload) downloadLabel = 'Starting download';
  else
    switch (activeDownload?.status) {
      case 'waiting_for_space':
        downloadLabel = 'Waiting for space';
        break;
      case 'queued':
        downloadLabel = 'Download queued';
        break;
      case 'downloading': {
        const activity =
          (downloadEvent?.mediaKind ?? activeDownload?.mediaKind) === 'video'
            ? 'Downloading video'
            : (downloadEvent?.mediaKind ?? activeDownload?.mediaKind) ===
                'audio'
              ? 'Downloading audio'
              : 'Downloading media';
        downloadLabel =
          downloadPercent === null
            ? activity
            : `${activity} ${Math.round(downloadPercent)}%`;
        break;
      }
      case 'processing':
        downloadLabel = 'Processing download';
        break;
      case 'interrupted':
        downloadLabel = 'Download paused';
        break;
      case 'failed': {
        const retryAt = activeDownload?.retryAt;
        downloadLabel = retryAt
          ? `Retrying at ${new Intl.DateTimeFormat(undefined, { timeStyle: 'short' }).format(Date.parse(retryAt))}`
          : 'Download failed';
        break;
      }
    }
  const readyDownloadTitle =
    downloadReady && activeDownload
      ? `${activeDownload.pinned ? 'Pinned · ' : ''}${formatBytes(activeDownload.fileSizeBytes)} local${downloadFormatLabel ? ` · ${downloadFormatLabel}` : ''}`
      : null;
  return {
    activeDownload,
    downloadReady,
    downloadPending,
    canPlay,
    canDownload,
    isBusy,
    tracksVideoProgress,
    inProgress,
    showPosition,
    remainingSeconds,
    primaryLabel,
    downloadPercent,
    downloadLabel,
    downloadFormatLabel,
    readyDownloadTitle,
    progressPercent: Math.min(100, Math.max(0, video.watchedPercentage)),
  };
}

export type YoutubeVideoPresentation = ReturnType<
  typeof youtubeVideoPresentation
>;
