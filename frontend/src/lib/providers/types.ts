export type Platform = 'youtube' | 'twitch' | 'kick';
export type Route =
  '/youtube' | '/twitch' | '/kick' | '/statistics' | '/settings';
export type Theme = 'system' | 'light' | 'dark';
export type YoutubeCookieBrowser =
  'none' | 'chrome' | 'edge' | 'firefox' | 'brave';
export type YoutubeWatchState = 'unwatched' | 'in_progress' | 'watched';
export type YoutubeSortField = 'date' | 'channel' | 'duration';
export type YoutubeSortDirection = 'asc' | 'desc';
export type YoutubeGrouping = 'smart' | 'day' | 'week' | 'month' | 'none';
export type YoutubeDurationFilter = 'any' | 'under_10' | '10_plus' | '30_plus';
export type YoutubePublishedFilter = 'any' | '7_days' | '30_days';
export type YoutubeDownloadFilter = 'all' | 'downloaded';
export type YoutubeBroadcastState = 'none' | 'upcoming' | 'live' | 'replay';
export type YoutubeDownloadQuality = '720p' | '1080p' | '1440p' | 'best';
export type YoutubeCardShortcut =
  | 'play_from_beginning'
  | 'play_signed_in'
  | 'download'
  | 'watch_toggle'
  | 'copy_url'
  | 'open_browser'
  | 'pin_download'
  | 'delete_download'
  | 'add_watch_later';
export type YoutubeWatchlistSortMode = 'manual' | 'date';

export interface YoutubeFeedQuery {
  search: string;
  watchStates: YoutubeWatchState[];
  includeShorts: boolean;
  includeLive: boolean;
  includeLiveReplays: boolean;
  includeUpcoming: boolean;
  channelId?: string | null;
  durationFilter: YoutubeDurationFilter;
  publishedFilter: YoutubePublishedFilter;
  sortField: YoutubeSortField;
  sortDirection: YoutubeSortDirection;
  grouping: YoutubeGrouping;
  downloadFilter: YoutubeDownloadFilter;
}

export interface AppError {
  category:
    | 'authentication'
    | 'api'
    | 'playback'
    | 'database'
    | 'configuration'
    | 'validation'
    | 'internal';
  code: string;
  message: string;
  action?: string | null;
  technical: string;
}

export interface AppSettings {
  theme: Theme;
  sidebarCollapsed: boolean;
  lastRoute: Route;
  mpvPath?: string | null;
  ytdlpPath?: string | null;
  streamlinkPath?: string | null;
  ffmpegPath?: string | null;
  youtubeCookieBrowser: YoutubeCookieBrowser;
  youtubeDownloadQuality: YoutubeDownloadQuality;
  youtubeDownloadStorageLimitGib: number;
  youtubeCardShortcuts: YoutubeCardShortcut[];
  watchedPercentageThreshold: number;
  remainingTimeThresholdSeconds: number;
  fadeWatchedCards: boolean;
}

export type AppPreferences = Omit<
  AppSettings,
  'theme' | 'sidebarCollapsed' | 'lastRoute'
>;
export type AppNavigation = Pick<AppSettings, 'sidebarCollapsed' | 'lastRoute'>;

export interface PlatformAccount {
  platform: 'youtube' | 'twitch';
  externalUserId: string;
  displayName?: string | null;
  avatarUrl?: string | null;
  connectedAt: string;
  updatedAt: string;
  lastValidatedAt?: string | null;
}

export type OAuthClientSource = 'saved' | 'environment' | 'missing';

export interface OAuthClientConfiguration {
  googleClientId?: string | null;
  googleClientSecretConfigured: boolean;
  googleSource: OAuthClientSource;
  twitchClientId?: string | null;
  twitchSource: OAuthClientSource;
  kickClientId?: string | null;
  kickClientSecretConfigured: boolean;
  kickSource: OAuthClientSource;
}

export interface SyncStatus {
  platform: Platform;
  operationId?: string | null;
  phase: string;
  isRefreshing: boolean;
  completed: number;
  total?: number | null;
  lastSuccessAt?: string | null;
  lastAttemptAt?: string | null;
  stale: boolean;
  quotaPausedUntil?: string | null;
  rateLimitedUntil?: string | null;
  error?: AppError | null;
}

export interface YoutubeVideo {
  videoId: string;
  channelId: string;
  channelName: string;
  channelThumbnailUrl?: string | null;
  title: string;
  thumbnailUrl?: string | null;
  publishedAt: string;
  durationSeconds?: number | null;
  isLive: boolean;
  isUpcoming: boolean;
  isLiveReplay: boolean;
  broadcastState: YoutubeBroadcastState;
  scheduledStartAt?: string | null;
  actualStartAt?: string | null;
  actualEndAt?: string | null;
  availabilityStatus?: string | null;
  positionSeconds: number;
  watchedPercentage: number;
  isWatched: boolean;
  completedAt?: string | null;
  download?: YoutubeDownload | null;
}

export interface YoutubeWatchlistItem {
  video: YoutubeVideo;
  addedAt: string;
  manualPosition: number;
  metadataPending?: boolean;
}

export interface YoutubeWatchlist {
  id: number;
  name: string;
  isDefault: boolean;
  autoDownload: boolean;
  autoRemoveWatched: boolean;
  sortMode: YoutubeWatchlistSortMode;
  sortDirection: YoutubeSortDirection;
  createdAt: string;
  updatedAt: string;
  items: YoutubeWatchlistItem[];
}

export interface YoutubeWatchlistUpdate {
  name: string;
  autoDownload: boolean;
  autoRemoveWatched: boolean;
  sortMode: YoutubeWatchlistSortMode;
  sortDirection: YoutubeSortDirection;
}

export interface YoutubeWatchlistAddResult {
  added: boolean;
  video: YoutubeVideo;
  autoDownloadError?: AppError | null;
}

export interface YoutubeDownload {
  videoId: string;
  status:
    | 'queued'
    | 'downloading'
    | 'processing'
    | 'ready'
    | 'interrupted'
    | 'failed';
  fileSizeBytes: number;
  pinned: boolean;
  quality: YoutubeDownloadQuality;
  heightPixels?: number | null;
  frameRate?: number | null;
  requestedAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
  lastPlayedAt?: string | null;
  errorCode?: string | null;
  errorMessage?: string | null;
}

export interface YoutubeDownloadEvent {
  download: YoutubeDownload;
  downloadedBytes: number;
  totalBytes?: number | null;
  etaSeconds?: number | null;
  mediaKind?: 'video' | 'audio' | 'media' | null;
  notice?: string | null;
}

export interface YoutubeVideoCounts {
  all: number;
  unwatched: number;
  inProgress: number;
  watched: number;
  shorts: number;
  live: number;
  liveReplays: number;
  upcoming: number;
  subscribedChannelCount: number;
}

export interface YoutubeVideoPage {
  items: YoutubeVideo[];
  page: number;
  pageSize: number;
  hasMore: boolean;
  counts: YoutubeVideoCounts;
  channels: YoutubeChannelOption[];
}

export interface YoutubeChannelOption {
  channelId: string;
  title: string;
}

export interface YoutubeProgressEvent {
  sessionId?: string | null;
  videoId: string;
  positionSeconds: number;
  durationSeconds?: number | null;
  watchedPercentage: number;
  isWatched: boolean;
}

// Added by the app after the backend progress event is applied to watchlists.
export interface YoutubeProgressUpdate extends YoutubeProgressEvent {
  manualWatchPending: boolean;
}

export interface TwitchLiveStream {
  streamId: string;
  broadcasterId: string;
  login: string;
  displayName: string;
  profileImageUrl?: string | null;
  title: string;
  gameId?: string | null;
  gameName?: string | null;
  thumbnailUrl?: string | null;
  viewerCount: number;
  startedAt: string;
  fetchedAt: string;
}

export interface KickChannel {
  slug: string;
  displayName: string;
  profilePictureUrl?: string | null;
  liveStream?: KickLiveStream | null;
}

export interface KickLiveStream {
  streamId: string;
  title: string;
  categoryName?: string | null;
  thumbnailUrl?: string | null;
  viewerCount: number;
  startedAt: string;
  languageCode?: string | null;
  hasMatureContent: boolean;
  tags: string[];
  fetchedAt: string;
}

export interface KickSnapshot {
  channels: KickChannel[];
}

export interface ExecutableDiagnostic {
  kind: string;
  detected: boolean;
  path?: string | null;
  version?: string | null;
  source?: string | null;
  warning?: string | null;
  updateAvailable?: boolean;
  error?: AppError | null;
}

export interface PlaybackDiagnostics {
  mpv: ExecutableDiagnostic;
  ytdlp: ExecutableDiagnostic;
  streamlink: ExecutableDiagnostic;
  ffmpeg: ExecutableDiagnostic;
  activeSessionCount: number;
}

export interface DataStats {
  databasePath: string;
  databaseSizeBytes: number;
  downloadDirectory: string;
  downloadStorageBytes: number;
  partialDownloadBytes: number;
  readyDownloadCount: number;
  cachedYoutubeVideoCount: number;
  savedProgressEntryCount: number;
  cachedTwitchChannelCount: number;
}

export interface PlaybackSession {
  sessionId: string;
  platform: Platform;
  mediaId: string;
  processId?: number | null;
  ipcEndpoint: string;
  startedAt: string;
  positionSeconds?: number | null;
  durationSeconds?: number | null;
  playbackState: string;
  connected: boolean;
  sourceKind: 'remote' | 'download';
}

export interface PlaybackEvent {
  sessionId: string;
  platform: Platform;
  mediaId: string;
  state: string;
  detail?: string | null;
}

export type StatisticsRange = '7d' | '30d' | '90d' | 'all';
export type StatisticsPlatform = 'all' | Platform;

export interface StatisticsQuery {
  range: StatisticsRange;
  platform: StatisticsPlatform;
  timezone: string;
}

export interface StatisticsActivityPoint {
  periodStart: string;
  youtubeSeconds: number;
  twitchSeconds: number;
  kickSeconds: number;
}

export interface StatisticsRhythmPoint {
  weekday: number;
  hour: number;
  youtubeSeconds: number;
  twitchSeconds: number;
  kickSeconds: number;
}

export interface StatisticsPlatformTotal {
  platform: Platform;
  activeSeconds: number;
}

export interface StatisticsChannel {
  platform: Platform;
  name: string;
  activeSeconds: number;
  watchedDays: number;
  contentCount: number;
}

export interface StatisticsYoutubeContentMix {
  uploads: number;
  liveReplays: number;
  shorts: number;
}

export interface StatisticsOverview {
  range: StatisticsRange;
  platform: StatisticsPlatform;
  interval: 'day' | 'week' | 'month';
  trackingStartedAt?: string | null;
  totalActiveSeconds: number;
  activeDays: number;
  periodDays: number;
  averageActiveSecondsPerDay: number;
  youtubeVideosStarted: number;
  youtubeVideosWatched: number;
  twitchChannelsWatched: number;
  kickChannelsWatched: number;
  activity: StatisticsActivityPoint[];
  rhythm: StatisticsRhythmPoint[];
  platformTotals: StatisticsPlatformTotal[];
  topChannels: StatisticsChannel[];
  youtubeContentMix: StatisticsYoutubeContentMix;
}

export interface MigrationFailure {
  databasePath: string;
  migration: number;
  error: AppError;
}

export interface BootstrapData {
  settings: AppSettings;
  youtubeAccount?: PlatformAccount | null;
  twitchAccount?: PlatformAccount | null;
  youtubeSync: SyncStatus;
  twitchSync: SyncStatus;
  kickSync: SyncStatus;
  playback: PlaybackDiagnostics;
  dataStats: DataStats;
  migrationFailure?: MigrationFailure | null;
  oauthClientConfiguration: OAuthClientConfiguration;
}

export interface AuthState {
  platform: 'youtube' | 'twitch';
  operationId?: string | null;
  status:
    | 'idle'
    | 'pending'
    | 'success'
    | 'cancelled'
    | 'denied'
    | 'expired'
    | 'callback_failed'
    | 'error';
  message?: string | null;
  userCode?: string | null;
  verificationUri?: string | null;
  expiresAt?: string | null;
}

export interface SyncEvent {
  operationId: string;
  status: SyncStatus;
}

export interface AuthEvent {
  operationId: string;
  status: AuthState['status'];
  message?: string | null;
  userCode?: string;
  verificationUri?: string;
  expiresAt?: string;
}
