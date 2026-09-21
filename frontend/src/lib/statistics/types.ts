export type StatisticsRange = '7d' | '30d' | '90d' | 'all';
export type StatisticsSource =
  'youtube' | 'twitch' | 'kick' | 'movies' | 'shows' | 'music';
export type StatisticsPlatform = 'all' | StatisticsSource;
export type StatisticsSeconds = Record<`${StatisticsSource}Seconds`, number>;
export type StatisticsActivityPoint = StatisticsSeconds & {
  periodStart: string;
};
export type StatisticsRhythmPoint = StatisticsSeconds & {
  weekday: number;
  hour: number;
};
export type StatisticsOverview = {
  range: StatisticsRange;
  platform: StatisticsPlatform;
  timezone: string;
  interval: 'day' | 'week' | 'month';
  trackingStartedAt: string | null;
  totalActiveSeconds: number;
  estimatedActiveSeconds: number;
  activeDays: number;
  periodDays: number;
  averageActiveSecondsPerDay: number;
  youtubeVideosStarted: number;
  youtubeVideosWatched: number;
  twitchChannelsWatched: number;
  kickChannelsWatched: number;
  moviesStarted: number;
  moviesWatched: number;
  episodesStarted: number;
  episodesWatched: number;
  showsWatched: number;
  tracksStarted: number;
  tracksCompleted: number;
  artistsListened: number;
  albumsListened: number;
  activity: StatisticsActivityPoint[];
  rhythm: StatisticsRhythmPoint[];
  platformTotals: { platform: StatisticsSource; activeSeconds: number }[];
  topChannels: {
    platform: StatisticsSource;
    id: string;
    name: string;
    activeSeconds: number;
    watchedDays: number;
    contentCount: number;
  }[];
  youtubeContentMix: { uploads: number; liveReplays: number; shorts: number };
  users: { id: string; username: string; activeSeconds: number }[];
};
