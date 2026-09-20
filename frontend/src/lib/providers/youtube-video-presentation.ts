import { formatBytes, formatClock } from './utils';
import type { YoutubeDownloadEvent, YoutubeVideo } from './types';

export function youtubeVideoPresentation(
  video: YoutubeVideo,
  context: {
    mpvReady: boolean;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    isPlaying: boolean;
    isLaunching: boolean;
    downloadBusy: boolean;
    downloadEvent?: YoutubeDownloadEvent;
  },
) {
  const {
    mpvReady,
    ytdlpReady,
    ffmpegReady,
    isPlaying,
    isLaunching,
    downloadBusy,
    downloadEvent,
  } = context;
  const activeDownload = downloadEvent?.download ?? video.download;
  const downloadReady = activeDownload?.status === 'ready';
  const downloadPending = ['queued', 'downloading', 'processing'].includes(
    activeDownload?.status ?? '',
  );
  const isBusy = isPlaying || isLaunching;
  const canPlay =
    mpvReady &&
    (downloadReady || ytdlpReady) &&
    (video.availabilityStatus !== 'unavailable' || downloadReady) &&
    !video.isUpcoming &&
    !isBusy;
  const canDownload =
    ytdlpReady &&
    ffmpegReady &&
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
  const downloadPercent = totalBytes
    ? Math.min(100, (downloadedBytes / totalBytes) * 100)
    : null;
  const downloadFormatLabel = activeDownload?.heightPixels
    ? `${activeDownload.heightPixels}p${activeDownload.frameRate ? Math.round(activeDownload.frameRate) : ''}`
    : null;
  let downloadLabel: string | null = null;
  if (downloadBusy && !activeDownload) downloadLabel = 'Starting download';
  else
    switch (activeDownload?.status) {
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
        downloadLabel = 'Finishing download';
        break;
      case 'interrupted':
        downloadLabel = 'Download paused';
        break;
      case 'failed':
        downloadLabel = 'Download failed';
        break;
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
