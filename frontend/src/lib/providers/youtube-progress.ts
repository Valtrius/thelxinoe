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

// Each playback report carries the position, so a page moves the video's card
// instead of reloading its lists. Live streams have no duration to measure.
export function playbackProgress(payload: {
  id?: string;
  media_id?: string;
  position?: number;
  duration?: number;
  watched?: boolean;
}): YoutubeProgressEvent | null {
  if (
    !payload.media_id?.startsWith('youtube:') ||
    payload.position === undefined ||
    !payload.duration
  )
    return null;
  return {
    sessionId: payload.id,
    videoId: payload.media_id.slice('youtube:'.length),
    positionSeconds: payload.position,
    durationSeconds: payload.duration,
    watchedPercentage: Math.min(
      100,
      Math.max(0, (payload.position / payload.duration) * 100),
    ),
    isWatched: payload.watched ?? false,
  };
}
