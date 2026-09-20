import type {
  YoutubeWatchState,
  YoutubeProgressEvent,
  YoutubeVideo,
  YoutubeVideoCounts,
} from './types';

type ProgressCategory = 'unwatched' | 'in_progress' | 'watched';

function progressCategory(
  watchedPercentage: number,
  isWatched: boolean,
): ProgressCategory {
  if (isWatched) return 'watched';
  return watchedPercentage > 1 ? 'in_progress' : 'unwatched';
}

function matchesFilter(
  video: YoutubeVideo,
  watchStates: YoutubeWatchState[],
): boolean {
  return watchStates.includes(
    progressCategory(video.watchedPercentage, video.isWatched),
  );
}

export function applyYoutubeProgress(
  videos: YoutubeVideo[],
  counts: YoutubeVideoCounts,
  watchStates: YoutubeWatchState[],
  event: YoutubeProgressEvent,
  retainFilteredVideo = false,
): { videos: YoutubeVideo[]; counts: YoutubeVideoCounts; found: boolean } {
  const index = videos.findIndex((video) => video.videoId === event.videoId);
  if (index < 0) return { videos, counts, found: false };

  const current = videos[index];
  const previousCategory = progressCategory(
    current.watchedPercentage,
    current.isWatched,
  );
  const nextCategory = progressCategory(
    event.watchedPercentage,
    event.isWatched,
  );
  const updated: YoutubeVideo = {
    ...current,
    positionSeconds: event.positionSeconds,
    watchedPercentage: event.watchedPercentage,
    isWatched: event.isWatched,
  };
  const nextVideos =
    retainFilteredVideo || matchesFilter(updated, watchStates)
      ? videos.map((video, videoIndex) =>
          videoIndex === index ? updated : video,
        )
      : videos.filter((_, videoIndex) => videoIndex !== index);

  if (previousCategory === nextCategory) {
    return { videos: nextVideos, counts, found: true };
  }
  return {
    videos: nextVideos,
    found: true,
    counts: {
      ...counts,
      unwatched:
        counts.unwatched +
        (nextCategory === 'unwatched' ? 1 : 0) -
        (previousCategory === 'unwatched' ? 1 : 0),
      inProgress:
        counts.inProgress +
        (nextCategory === 'in_progress' ? 1 : 0) -
        (previousCategory === 'in_progress' ? 1 : 0),
      watched:
        counts.watched +
        (nextCategory === 'watched' ? 1 : 0) -
        (previousCategory === 'watched' ? 1 : 0),
    },
  };
}
