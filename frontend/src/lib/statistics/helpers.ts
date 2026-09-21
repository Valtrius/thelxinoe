import type {
  StatisticsPlatform,
  StatisticsOverview,
  StatisticsSource,
  StatisticsSeconds,
} from './types';

export const sources = [
  {
    value: 'youtube',
    label: 'YouTube',
    field: 'youtubeSeconds',
    color: '#ff747b',
  },
  {
    value: 'twitch',
    label: 'Twitch',
    field: 'twitchSeconds',
    color: '#b9a4ff',
  },
  { value: 'kick', label: 'Kick', field: 'kickSeconds', color: '#53fc18' },
  { value: 'movies', label: 'Films', field: 'moviesSeconds', color: '#65bfff' },
  { value: 'shows', label: 'Shows', field: 'showsSeconds', color: '#ffb45c' },
  { value: 'music', label: 'Music', field: 'musicSeconds', color: '#eb8ccd' },
] as const;

export function visibleSources(platform: StatisticsPlatform) {
  return sources.filter(
    (source) => platform === 'all' || source.value === platform,
  );
}

export function selectedStatisticsSeconds(
  point: StatisticsSeconds,
  platform: StatisticsPlatform,
): number {
  return visibleSources(platform).reduce(
    (total, source) => total + point[source.field],
    0,
  );
}

export function formatWatchDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds <= 0) return '0m';
  const roundedMinutes = Math.max(1, Math.round(totalSeconds / 60));
  const hours = Math.floor(roundedMinutes / 60);
  const minutes = roundedMinutes % 60;
  if (hours === 0) return `${minutes}m`;
  if (minutes === 0) return `${hours}h`;
  return `${hours}h ${minutes}m`;
}

export function watchedCompletionPercent(
  started: number,
  watched: number,
): number {
  if (started <= 0 || watched <= 0) return 0;
  return Math.min(100, Math.round((watched / started) * 100));
}

export function heatLevel(value: number, maximum: number): number {
  if (value <= 0 || maximum <= 0) return 0;
  return Math.max(1, Math.min(5, Math.ceil((value / maximum) * 5)));
}

export type GridNavigationKey =
  'ArrowLeft' | 'ArrowRight' | 'ArrowUp' | 'ArrowDown' | 'Home' | 'End';

export function gridNavigationTarget(
  currentIndex: number,
  key: GridNavigationKey,
  rowCount: number,
  columnCount: number,
): number {
  const lastIndex = rowCount * columnCount - 1;
  const boundedIndex = Math.max(0, Math.min(currentIndex, lastIndex));
  const row = Math.floor(boundedIndex / columnCount);
  const column = boundedIndex % columnCount;

  switch (key) {
    case 'ArrowLeft':
      return column > 0 ? boundedIndex - 1 : boundedIndex;
    case 'ArrowRight':
      return column < columnCount - 1 ? boundedIndex + 1 : boundedIndex;
    case 'ArrowUp':
      return row > 0 ? boundedIndex - columnCount : boundedIndex;
    case 'ArrowDown':
      return row < rowCount - 1 ? boundedIndex + columnCount : boundedIndex;
    case 'Home':
      return row * columnCount;
    case 'End':
      return row * columnCount + columnCount - 1;
  }
}

export function chartTooltipPosition(
  clientX: number,
  clientY: number,
  viewportWidth: number,
) {
  const width = 230;
  const height = 105;
  const left = Math.max(10, Math.min(clientX + 14, viewportWidth - width - 10));
  const above = clientY - height - 12;
  return { left, top: above >= 10 ? above : clientY + 16 };
}

export function statisticsPlatformMetrics(overview: StatisticsOverview | null) {
  const metrics: Record<StatisticsSource, { seconds: number; share: number }> =
    {
      youtube: { seconds: 0, share: 0 },
      twitch: { seconds: 0, share: 0 },
      kick: { seconds: 0, share: 0 },
      movies: { seconds: 0, share: 0 },
      shows: { seconds: 0, share: 0 },
      music: { seconds: 0, share: 0 },
    };
  for (const total of overview?.platformTotals ?? []) {
    metrics[total.platform].seconds = total.activeSeconds;
  }
  for (const { value: platform } of sources) {
    metrics[platform].share =
      overview && overview.totalActiveSeconds > 0
        ? (metrics[platform].seconds / overview.totalActiveSeconds) * 100
        : 0;
  }
  return metrics;
}

export function completionDetails(
  overview: StatisticsOverview | null,
  platform: StatisticsPlatform,
) {
  const yt = overview?.youtubeContentMix;
  const youtube = {
    title: 'YOUTUBE LIBRARY',
    label: 'videos',
    started: overview?.youtubeVideosStarted ?? 0,
    watched: overview?.youtubeVideosWatched ?? 0,
    items: [
      { label: 'Uploads', value: yt?.uploads ?? 0 },
      { label: 'Live replays', value: yt?.liveReplays ?? 0 },
      { label: 'Shorts', value: yt?.shorts ?? 0 },
    ],
    note: '',
  };
  const local = {
    movies: {
      title: 'FILM LIBRARY',
      label: 'films',
      started: overview?.moviesStarted ?? 0,
      watched: overview?.moviesWatched ?? 0,
      note: '',
    },
    shows: {
      title: 'SHOW LIBRARY',
      label: 'episodes',
      started: overview?.episodesStarted ?? 0,
      watched: overview?.episodesWatched ?? 0,
      note: `${overview?.showsWatched ?? 0} shows started`,
    },
    music: {
      title: 'MUSIC LIBRARY',
      label: 'tracks',
      started: overview?.tracksStarted ?? 0,
      watched: overview?.tracksCompleted ?? 0,
      note: `${overview?.artistsListened ?? 0} artists · ${overview?.albumsListened ?? 0} albums`,
    },
  };
  if (platform === 'movies' || platform === 'shows' || platform === 'music') {
    const value = local[platform];
    return {
      ...value,
      items: [
        { label: 'Completed', value: value.watched },
        {
          label: 'In progress',
          value: Math.max(0, value.started - value.watched),
        },
      ],
    };
  }
  if (
    platform === 'all' &&
    Object.values(local).some((item) => item.started > 0)
  ) {
    return {
      title: 'LIBRARY COMPLETION',
      label: 'items',
      started:
        youtube.started +
        local.movies.started +
        local.shows.started +
        local.music.started,
      watched:
        youtube.watched +
        local.movies.watched +
        local.shows.watched +
        local.music.watched,
      items: [
        { label: 'YouTube', value: youtube.watched },
        { label: 'Films', value: local.movies.watched },
        { label: 'Episodes', value: local.shows.watched },
        { label: 'Tracks', value: local.music.watched },
      ],
      note: '',
    };
  }
  return youtube;
}
