import type {
  YoutubeDurationFilter,
  YoutubeDownloadFilter,
  YoutubeGrouping,
  YoutubePublishedFilter,
  YoutubeSortDirection,
  YoutubeSortField,
  YoutubeWatchState,
} from './types';

export interface YoutubeFeedPreferences {
  searchText: string;
  watchStates: YoutubeWatchState[];
  showShorts: boolean;
  showLive: boolean;
  showLiveReplays: boolean;
  showUpcoming: boolean;
  channelId: string;
  durationFilter: YoutubeDurationFilter;
  publishedFilter: YoutubePublishedFilter;
  sortField: YoutubeSortField;
  sortDirection: YoutubeSortDirection;
  grouping: YoutubeGrouping;
  downloadFilter: YoutubeDownloadFilter;
}

export const validWatchStates: YoutubeWatchState[] = [
  'unwatched',
  'in_progress',
  'watched',
];

export function defaultYoutubeFeedPreferences(): YoutubeFeedPreferences {
  return {
    searchText: '',
    watchStates: [...validWatchStates],
    showShorts: false,
    showLive: true,
    showLiveReplays: false,
    showUpcoming: false,
    channelId: '',
    durationFilter: 'any',
    publishedFilter: 'any',
    sortField: 'date',
    sortDirection: 'desc',
    grouping: 'smart',
    downloadFilter: 'all',
  };
}

function enumValue<T extends string>(
  value: unknown,
  allowed: readonly T[],
  fallback: T,
): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

export function parseYoutubeFeedPreferences(
  serialized: string | null,
): YoutubeFeedPreferences {
  const fallback = defaultYoutubeFeedPreferences();
  if (!serialized) return fallback;

  try {
    const saved = JSON.parse(serialized) as Record<string, unknown>;
    const savedWatchStates = saved.watchStates;
    const watchStates = Array.isArray(savedWatchStates)
      ? validWatchStates.filter((state) => savedWatchStates.includes(state))
      : fallback.watchStates;
    return {
      searchText:
        typeof saved.searchText === 'string'
          ? saved.searchText.slice(0, 200)
          : fallback.searchText,
      watchStates:
        watchStates.length > 0 ? watchStates : [...fallback.watchStates],
      showShorts:
        typeof saved.showShorts === 'boolean'
          ? saved.showShorts
          : fallback.showShorts,
      showLive:
        typeof saved.showLive === 'boolean'
          ? saved.showLive
          : fallback.showLive,
      showLiveReplays:
        typeof saved.showLiveReplays === 'boolean'
          ? saved.showLiveReplays
          : fallback.showLiveReplays,
      showUpcoming:
        typeof saved.showUpcoming === 'boolean'
          ? saved.showUpcoming
          : fallback.showUpcoming,
      channelId:
        typeof saved.channelId === 'string'
          ? saved.channelId
          : fallback.channelId,
      durationFilter: enumValue(
        saved.durationFilter,
        ['any', 'under_10', '10_plus', '30_plus'],
        fallback.durationFilter,
      ),
      publishedFilter: enumValue(
        saved.publishedFilter,
        ['any', '7_days', '30_days'],
        fallback.publishedFilter,
      ),
      sortField: enumValue(
        saved.sortField,
        ['date', 'channel', 'duration'],
        fallback.sortField,
      ),
      sortDirection: enumValue(
        saved.sortDirection,
        ['asc', 'desc'],
        fallback.sortDirection,
      ),
      grouping: enumValue(
        saved.grouping,
        ['smart', 'day', 'week', 'month', 'none'],
        fallback.grouping,
      ),
      downloadFilter: enumValue(
        saved.downloadFilter,
        ['all', 'downloaded'],
        fallback.downloadFilter,
      ),
    };
  } catch {
    return fallback;
  }
}
