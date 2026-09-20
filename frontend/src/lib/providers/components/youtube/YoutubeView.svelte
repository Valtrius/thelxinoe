<script lang="ts">
  import {
    captureScrollAnchor,
    restoreScrollAnchor,
    type ScrollAnchor,
  } from '../../scroll-anchor';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { onDestroy, tick, untrack } from 'svelte';
  import {
    manualWatchFilterDelayMs,
    type YoutubeWatchlistController,
  } from '../../youtube-watchlist-controller.svelte';
  import { createYoutubeFeed } from '../../youtube-feed-controller.svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import { api, normalizeError, preferences } from '../../api';
  import { createCardGridWheelHandler } from '../../card-grid-wheel';
  import { scaleYoutubeMediaScope } from '../../card-grid-zoom';
  import { createLayoutMotion, type LayoutSnapshot } from '../../layout-motion';
  import { dismissToastByKey, showToast } from '../../toasts';
  import type {
    AppError,
    AuthState,
    PlatformAccount,
    PlaybackDiagnostics,
    PlaybackSession,
    SyncStatus,
    YoutubeCardShortcut,
    YoutubeDownload,
    YoutubeDownloadEvent,
    YoutubeFeedQuery,
    YoutubeGrouping,
    YoutubeProgressUpdate,
    YoutubeVideo,
  } from '../../types';
  import {
    isPlaybackLaunching,
    localDateTime,
    relativeTime,
    youtubeUrl,
  } from '../../utils';
  import { applyYoutubeProgress } from '../../youtube-progress';
  import { groupYoutubeVideos } from '../../youtube-feed';
  import {
    validWatchStates as allWatchStates,
    parseYoutubeFeedPreferences,
  } from '../../youtube-feed-preferences';
  import { youtubeVideoIdFromInput } from '../../youtube-video-input';
  import Button from '../ui/Button.svelte';
  import EmptyState from '../ui/EmptyState.svelte';
  import YoutubeFeedToolbar from './YoutubeFeedToolbar.svelte';
  import VideoGrid from './VideoGrid.svelte';
  import WatchlistSidebar from './WatchlistSidebar.svelte';

  let {
    account,
    watchlistController,
    syncStatus,
    playback,
    activeSessions,
    cardColumns,
    fadeWatchedCards,
    youtubeCardShortcuts,
    signedInPlaybackAvailable,
    authState,
    clientConfigured,
    dataRevision,
    youtubeProgress,
    youtubeDownload,
    youtubeDownloadRemoval,
    onAuthStarted,
    onNavigateSettings,
    onAccountChanged,
    onYoutubeCardShortcutsChanged,
    onCardColumnsChange,
  }: {
    account?: PlatformAccount | null;
    watchlistController: YoutubeWatchlistController;
    syncStatus: SyncStatus;
    playback: PlaybackDiagnostics;
    activeSessions: PlaybackSession[];
    cardColumns: number;
    fadeWatchedCards: boolean;
    youtubeCardShortcuts: YoutubeCardShortcut[];
    signedInPlaybackAvailable: boolean;
    authState: AuthState;
    clientConfigured: boolean;
    dataRevision: number;
    youtubeProgress: YoutubeProgressUpdate | null;
    youtubeDownload: YoutubeDownloadEvent | null;
    youtubeDownloadRemoval: { videoId: string; sequence: number } | null;
    onAuthStarted: (operationId: string) => void;
    onNavigateSettings: (section: 'youtube' | 'tools') => void;
    onAccountChanged: () => void;
    onYoutubeCardShortcutsChanged: (shortcuts: YoutubeCardShortcut[]) => void;
    onCardColumnsChange: (columns: number) => void;
  } = $props();
  const videoActions = $derived<YoutubeVideoActions>({
    onPlay: (video) => play(video),
    onBeginning: (video) => play(video, true),
    onSignedIn: (video) => play(video, false, true),
    onDownload: (video) => startDownload(video),
    onDownloadSignedIn: (video) => startDownload(video, true),
    onCancelDownload: cancelDownload,
    onPinDownload: (video) => setDownloadPinned(video, true),
    onUnpinDownload: (video) => setDownloadPinned(video, false),
    onDeleteDownload: deleteDownload,
    onMarkWatched: (video) => void markWatched(video),
    onMarkUnwatched: (video) => void markUnwatched(video),
    onCopy: copy,
    onOpen: open,
    onAddToWatchlist: (video, watchlistId) =>
      void watchlistController.addToWatchlist(video, watchlistId),
    onRemoveFromWatchlist: (video, watchlistId) =>
      void watchlistController.removeFromWatchlist(watchlistId, video),
    onYoutubeCardShortcutsChanged: onYoutubeCardShortcutsChanged,
  });

  const feedPreferencesKey = 'youtube-feed-layout';
  const watchlistSidebarKey = 'youtube-watchlist-sidebar-open';
  const initialPreferences = parseYoutubeFeedPreferences(
    preferences.getItem(feedPreferencesKey),
  );

  let filters = $state(initialPreferences);
  let search = $state(
    youtubeVideoIdFromInput(initialPreferences.searchText)
      ? ''
      : initialPreferences.searchText.trim(),
  );

  let error = $state<AppError | null>(null);
  let refreshing = $state(false);
  let watchlistSidebarOpen = $state(
    preferences.getItem(watchlistSidebarKey) === 'true',
  );

  let actionBusy = $state(false);
  let lastSyncErrorFingerprint = $state<string | null>(null);
  let observedDataRevision = $state<number | null>(null);
  let appliedProgress = $state<YoutubeProgressUpdate | null>(null);
  let missingProgressFingerprint = $state<string | null>(null);
  let feedScroll = $state<HTMLElement | null>(null);
  const layoutMotion = createLayoutMotion();
  const connectLayoutMotion = layoutMotion.connect;
  const pendingVideoIds = new SvelteSet<string>();

  const downloadProgress = new SvelteMap<string, YoutubeDownloadEvent>();

  const deferredWatchedRemovals = new SvelteMap<
    string,
    { timer: number; deadline: number }
  >();

  onDestroy(() => {
    feed.dispose();
    for (const removal of deferredWatchedRemovals.values()) {
      window.clearTimeout(removal.timer);
    }
    deferredWatchedRemovals.clear();
  });
  const feed = createYoutubeFeed({
    get key() {
      return feedKey;
    },
    get query() {
      return feedQuery;
    },
    get connected() {
      return Boolean(account);
    },
    acceptChannels: (channels) => {
      if (
        filters.channelId &&
        !channels.some((channel) => channel.channelId === filters.channelId)
      ) {
        filters.channelId = '';
        return false;
      }
      return true;
    },
    prepareLayout: (reset) => {
      const anchor = reset
        ? null
        : captureScrollAnchor(feedScroll, 'data-video-id');
      const snapshot = layoutMotion.capture();
      return () => playLayoutMotion(snapshot, anchor);
    },
    onError: (caught) => {
      error = caught;
    },
  });
  const directVideoId = $derived(youtubeVideoIdFromInput(filters.searchText));
  const defaultWatchlist = $derived(
    watchlistController.watchlists.find((watchlist) => watchlist.isDefault),
  );
  const directInWatchlist = $derived(
    defaultWatchlist?.items.some(
      (item) => item.video.videoId === directVideoId,
    ) ?? false,
  );
  const directWatchlistBusy = $derived(
    Boolean(
      defaultWatchlist &&
      directVideoId &&
      watchlistController.isAddingToWatchlist(
        defaultWatchlist.id,
        directVideoId,
      ),
    ),
  );
  const playbackReady = $derived(
    playback.mpv.detected && playback.ytdlp.detected,
  );
  const activeVideoIds = $derived(
    new Set(
      activeSessions
        .filter((session) => session.platform === 'youtube')
        .map((session) => session.mediaId),
    ),
  );
  const launchingVideoIds = $derived(
    new Set([
      ...pendingVideoIds,
      ...activeSessions
        .filter(
          (session) =>
            session.platform === 'youtube' &&
            isPlaybackLaunching(session.playbackState),
        )
        .map((session) => session.mediaId),
    ]),
  );
  const directLaunchState = $derived.by(() => {
    if (!directVideoId) return null;
    if (pendingVideoIds.has(directVideoId)) return 'launching';
    if (activeVideoIds.has(directVideoId)) return 'active';
    return 'ready';
  });
  const authFailureTitle = $derived.by(() => {
    switch (authState.status) {
      case 'cancelled':
        return 'Google sign-in cancelled';
      case 'denied':
        return 'YouTube access denied';
      case 'expired':
        return 'Google sign-in expired';
      case 'callback_failed':
        return 'Google callback failed';
      default:
        return 'Google sign-in failed';
    }
  });
  const feedQuery = $derived<YoutubeFeedQuery>({
    search,
    watchStates: filters.watchStates,
    includeShorts: filters.showShorts,
    includeLive: filters.showLive,
    includeLiveReplays: filters.showLiveReplays,
    includeUpcoming: filters.showUpcoming,
    channelId: filters.channelId || null,
    durationFilter: filters.durationFilter,
    publishedFilter: filters.publishedFilter,
    sortField: filters.sortField,
    sortDirection: filters.sortDirection,
    grouping: filters.grouping,
    downloadFilter: filters.downloadFilter,
  });
  const feedKey = $derived(
    `${account?.externalUserId ?? 'disconnected'}:${JSON.stringify(feedQuery)}`,
  );
  const groupedVideos = $derived(
    groupYoutubeVideos(feed.videos, filters.grouping),
  );
  const hasActiveFilters = $derived(
    search.length > 0 ||
      filters.watchStates.length < allWatchStates.length ||
      filters.showShorts ||
      !filters.showLive ||
      filters.showLiveReplays ||
      filters.showUpcoming ||
      filters.channelId !== '' ||
      filters.durationFilter !== 'any' ||
      filters.publishedFilter !== 'any' ||
      filters.downloadFilter !== 'all',
  );
  const quotaReset = $derived(localDateTime(syncStatus.quotaPausedUntil));

  $effect(() => {
    const serialized = $preferences[feedPreferencesKey];
    if (serialized)
      untrack(() => {
        const remote = parseYoutubeFeedPreferences(serialized);
        const current = {
          ...filters,
          searchText: directVideoId ? '' : filters.searchText,
        };
        if (JSON.stringify(remote) !== JSON.stringify(current))
          filters = remote;
      });
  });
  $effect(() => {
    const saved = $preferences[watchlistSidebarKey];
    if (saved !== undefined) watchlistSidebarOpen = saved === 'true';
  });

  $effect(() => {
    const value = filters.searchText;
    const timeout = window.setTimeout(
      () => (search = youtubeVideoIdFromInput(value) ? '' : value.trim()),
      250,
    );
    return () => window.clearTimeout(timeout);
  });

  $effect(() => {
    preferences.setItem(
      feedPreferencesKey,
      JSON.stringify({
        searchText: directVideoId ? '' : filters.searchText,
        watchStates: filters.watchStates,
        showShorts: filters.showShorts,
        showLive: filters.showLive,
        showLiveReplays: filters.showLiveReplays,
        showUpcoming: filters.showUpcoming,
        channelId: filters.channelId,
        durationFilter: filters.durationFilter,
        publishedFilter: filters.publishedFilter,
        sortField: filters.sortField,
        sortDirection: filters.sortDirection,
        grouping: filters.grouping,
        downloadFilter: filters.downloadFilter,
      }),
    );
  });

  $effect(() => {
    preferences.setItem(watchlistSidebarKey, String(watchlistSidebarOpen));
  });

  const handleCardGridWheel = createCardGridWheelHandler({
    anchorAttribute: 'data-video-id',
    motion: layoutMotion,
    getColumns: () => cardColumns,
    setColumns: (columns) => onCardColumnsChange(columns),
  });

  $effect(() => {
    const event = youtubeDownload;
    if (!event) return;
    untrack(() => {
      if (event.notice) {
        showToast({
          key: 'youtube-download-notice',
          tone: 'warning',
          title: 'YouTube download',
          message: event.notice,
        });
      }
      const videoId = event.download.videoId;
      if (event.download.status === 'failed') {
        const openSettings =
          event.download.errorCode === 'youtube_cookie_browser_missing' ||
          event.download.errorCode === 'youtube_cookie_browser_locked';
        showToast({
          key: 'youtube-download-error',
          tone: 'error',
          title: 'YouTube download failed',
          message:
            event.download.errorMessage ?? 'The video could not be downloaded.',
          actionLabel: openSettings ? 'YouTube settings' : null,
          onAction: openSettings ? () => onNavigateSettings('youtube') : null,
          durationMs: null,
        });
      } else {
        dismissToastByKey('youtube-download-error');
      }
      downloadProgress.set(videoId, event);
      watchlistController.updateWatchlistVideo(videoId, (video) => ({
        ...video,
        download: event.download,
      }));
      const index = feed.videos.findIndex((video) => video.videoId === videoId);
      if (index >= 0) {
        if (
          filters.downloadFilter === 'downloaded' &&
          event.download.status !== 'ready'
        ) {
          feed.removeRetainedItem(index);
          feed.videos = feed.videos.filter(
            (video) => video.videoId !== videoId,
          );
        } else {
          feed.videos = feed.videos.map((video) =>
            video.videoId === videoId
              ? { ...video, download: event.download }
              : video,
          );
        }
      } else if (
        filters.downloadFilter === 'downloaded' &&
        event.download.status === 'ready'
      ) {
        void feed.refreshLoadedVideos(feedKey);
      }
    });
  });

  $effect(() => {
    const removal = youtubeDownloadRemoval;
    if (!removal) return;
    untrack(() => {
      downloadProgress.delete(removal.videoId);
      watchlistController.updateWatchlistVideo(removal.videoId, (video) => ({
        ...video,
        download: null,
      }));
      const index = feed.videos.findIndex(
        (video) => video.videoId === removal.videoId,
      );
      if (index < 0) return;
      if (filters.downloadFilter === 'downloaded') {
        feed.removeRetainedItem(index);
        feed.videos = feed.videos.filter(
          (video) => video.videoId !== removal.videoId,
        );
      } else {
        feed.videos = feed.videos.map((video) =>
          video.videoId === removal.videoId
            ? { ...video, download: null }
            : video,
        );
      }
    });
  });

  $effect(() => {
    const requestKey = feedKey;
    untrack(() => void feed.loadVideos(true, requestKey));
  });

  $effect(() => {
    const revision = dataRevision;
    if (observedDataRevision === null) {
      observedDataRevision = revision;
      return;
    }
    if (revision === observedDataRevision) return;
    observedDataRevision = revision;
    const requestKey = feedKey;
    untrack(() => void feed.refreshLoadedVideos(requestKey));
  });

  $effect(() => {
    const progress = youtubeProgress;
    if (!progress || progress === appliedProgress) return;
    appliedProgress = progress;
    const manualWatchPending = progress.manualWatchPending ?? false;
    const deferFilterRemoval =
      progress.isWatched &&
      !filters.watchStates.includes('watched') &&
      (manualWatchPending || deferredWatchedRemovals.has(progress.videoId));
    if (!progress.isWatched) cancelDeferredWatchedRemoval(progress.videoId);
    const updated = untrack(() =>
      applyYoutubeProgress(
        feed.videos,
        feed.counts,
        filters.watchStates,
        progress,
        deferFilterRemoval,
      ),
    );
    if (!updated.found) {
      const category = progress.isWatched
        ? 'watched'
        : progress.watchedPercentage > 1
          ? 'in_progress'
          : 'unwatched';
      const fingerprint = `${progress.videoId}:${category}`;
      if (fingerprint !== missingProgressFingerprint) {
        missingProgressFingerprint = fingerprint;
        const requestKey = feedKey;
        untrack(() => void feed.refreshLoadedVideos(requestKey));
      }
      return;
    }
    missingProgressFingerprint = null;
    const removedVideo = updated.videos.length < feed.videos.length;
    const layoutSnapshot = removedVideo ? layoutMotion.capture() : null;
    if (removedVideo) {
      feed.removeRetainedItem(
        feed.videos.findIndex((video) => video.videoId === progress.videoId),
      );
    }
    feed.videos = updated.videos;
    feed.counts = updated.counts;
    if (layoutSnapshot) untrack(() => void playLayoutMotion(layoutSnapshot));
    if (deferFilterRemoval && manualWatchPending) {
      scheduleDeferredWatchedRemoval(progress.videoId);
    }
  });

  $effect(() => {
    if (account && !playbackReady) {
      showToast({
        key: 'youtube-playback-missing',
        tone: 'warning',
        title: 'Playback dependencies missing',
        message: 'Configure the Windows player in Settings to play videos.',
        actionLabel: 'Player settings',
        onAction: () => onNavigateSettings('tools'),
        durationMs: null,
      });
    } else {
      dismissToastByKey('youtube-playback-missing');
    }
  });

  $effect(() => {
    const watchlistError = watchlistController.error;
    if (watchlistError) {
      showToast({
        key: 'youtube-watchlist-error',
        tone: 'error',
        title: 'Watchlist change failed',
        message: watchlistError.message,
        durationMs: null,
      });
    } else dismissToastByKey('youtube-watchlist-error');
  });

  $effect(() => {
    if (error) {
      showToast({
        key: 'youtube-action-error',
        tone: 'error',
        title: 'YouTube action failed',
        message: error.message,
        detail: error.technical,
        actionLabel: error.action ? 'Retry feed' : null,
        onAction: error.action ? () => feed.loadVideos(true) : null,
        durationMs: null,
      });
    } else {
      dismissToastByKey('youtube-action-error');
    }
  });

  $effect(() => {
    const syncError = syncStatus.error;
    if (!syncError) {
      lastSyncErrorFingerprint = null;
      dismissToastByKey('youtube-sync-error');
      return;
    }
    const fingerprint = `${syncError.code}:${syncError.message}:${syncError.technical}`;
    if (fingerprint === lastSyncErrorFingerprint) return;
    lastSyncErrorFingerprint = fingerprint;
    const cachedVideosMessage =
      feed.videos.length > 0 &&
      !syncError.message.toLowerCase().includes('cached video')
        ? ' Cached videos remain available.'
        : '';
    showToast({
      key: 'youtube-sync-error',
      tone: syncStatus.quotaPausedUntil ? 'warning' : 'error',
      title: syncStatus.quotaPausedUntil
        ? 'YouTube quota paused'
        : 'YouTube refresh failed',
      message: `${syncError.message}${syncStatus.quotaPausedUntil ? ` Requests resume ${quotaReset}.` : ''}${cachedVideosMessage}${feed.videos.length > 0 ? ` Last good sync ${relativeTime(syncStatus.lastSuccessAt)}.` : ''}`,
      detail: syncError.technical,
      actionLabel: syncStatus.quotaPausedUntil ? null : 'Retry',
      onAction: syncStatus.quotaPausedUntil ? null : () => refresh('normal'),
      durationMs: syncStatus.quotaPausedUntil ? null : 10_000,
    });
  });

  function cancelDeferredWatchedRemoval(videoId: string): number | null {
    const removal = deferredWatchedRemovals.get(videoId);
    if (!removal) return null;
    window.clearTimeout(removal.timer);
    deferredWatchedRemovals.delete(videoId);
    return Math.max(0, removal.deadline - Date.now());
  }

  function scheduleDeferredWatchedRemoval(
    videoId: string,
    delayMs = manualWatchFilterDelayMs,
  ) {
    cancelDeferredWatchedRemoval(videoId);
    const timer = window.setTimeout(() => {
      deferredWatchedRemovals.delete(videoId);
      const index = feed.videos.findIndex((video) => video.videoId === videoId);
      if (
        index < 0 ||
        !feed.videos[index].isWatched ||
        filters.watchStates.includes('watched')
      ) {
        return;
      }
      const layoutSnapshot = layoutMotion.capture();
      feed.removeRetainedItem(index);
      feed.videos = feed.videos.filter((video) => video.videoId !== videoId);
      void playLayoutMotion(layoutSnapshot);
    }, delayMs);
    deferredWatchedRemovals.set(videoId, {
      timer,
      deadline: Date.now() + delayMs,
    });
  }

  async function playLayoutMotion(
    snapshot: LayoutSnapshot | null,
    anchor: ScrollAnchor | null = null,
  ) {
    await tick();
    restoreScrollAnchor(feedScroll, 'data-video-id', anchor);
    layoutMotion.play(snapshot);
  }

  async function setGrouping(nextGrouping: YoutubeGrouping) {
    if (nextGrouping === filters.grouping) return;
    const layoutSnapshot = layoutMotion.capture();
    filters.grouping = nextGrouping;
    await playLayoutMotion(layoutSnapshot);
  }

  async function loadNewestVideos() {
    await feed.loadVideos(true);
    if (feed.firstPage === 0 && feedScroll) feedScroll.scrollTop = 0;
  }

  async function connect() {
    error = null;
    try {
      onAuthStarted(await api.connectYoutube());
    } catch (caught) {
      error = normalizeError(caught);
      if (error.category === 'authentication') onAccountChanged();
    }
  }

  async function cancelAuth() {
    if (!authState.operationId) return;
    try {
      await api.cancelYoutubeAuth(authState.operationId);
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function refresh(mode: 'normal' | 'full') {
    if (refreshing) return;
    refreshing = true;
    error = null;
    try {
      await feed.refreshLoadedVideos();
      await watchlistController.loadWatchlists();
      if (!syncStatus.isRefreshing) await api.syncYoutube(mode);
    } catch (caught) {
      error = normalizeError(caught);
      if (error.category === 'authentication') onAccountChanged();
    } finally {
      refreshing = false;
    }
  }

  async function launchVideo(
    videoId: string,
    startMode: 'resume' | 'beginning',
    signedIn = false,
  ): Promise<boolean> {
    if (pendingVideoIds.has(videoId) || activeVideoIds.has(videoId))
      return false;
    pendingVideoIds.add(videoId);
    error = null;
    try {
      await api.launchYoutube(videoId, startMode, signedIn);
      return true;
    } catch (caught) {
      error = normalizeError(caught);
      return false;
    } finally {
      pendingVideoIds.delete(videoId);
    }
  }

  async function play(
    video: YoutubeVideo,
    beginning = false,
    signedIn = false,
  ) {
    await launchVideo(
      video.videoId,
      beginning || video.isWatched
        ? 'beginning'
        : video.positionSeconds > 1
          ? 'resume'
          : 'beginning',
      signedIn,
    );
  }

  async function submitSearch(event: SubmitEvent) {
    event.preventDefault();
    if (!directVideoId || !playbackReady) return;
    if (await launchVideo(directVideoId, 'resume')) {
      filters.searchText = '';
      search = '';
    }
  }

  async function addDirectVideoToWatchlist() {
    const videoId = directVideoId;
    const watchlist = defaultWatchlist;
    if (!videoId || !watchlist || directWatchlistBusy || directInWatchlist)
      return;
    const adding = watchlistController.addToWatchlist(videoId, watchlist.id);
    filters.searchText = '';
    search = '';
    if (await adding) {
      showToast({
        key: `direct-watchlist-${videoId}`,
        tone: 'success',
        title: `Added to ${watchlist.name}`,
        message: 'The video is saved in your watchlist.',
      });
    }
  }

  async function mutate(action: () => Promise<void>): Promise<boolean> {
    actionBusy = true;
    error = null;
    try {
      await action();
      return true;
    } catch (caught) {
      error = normalizeError(caught);
      return false;
    } finally {
      actionBusy = false;
    }
  }

  async function markWatched(video: YoutubeVideo) {
    watchlistController.pendingManualWatchedVideoIds.add(video.videoId);
    if (!(await mutate(() => api.markYoutubeWatched(video.videoId)))) {
      watchlistController.pendingManualWatchedVideoIds.delete(video.videoId);
    }
  }

  async function markUnwatched(video: YoutubeVideo) {
    watchlistController.pendingManualWatchedVideoIds.delete(video.videoId);
    const remainingDelay = cancelDeferredWatchedRemoval(video.videoId);
    const remainingWatchlistDelay =
      watchlistController.invalidateDeferredWatchlistRemoval(video.videoId);
    const succeeded = await mutate(() =>
      api.markYoutubeUnwatched(video.videoId),
    );
    if (!succeeded) {
      if (remainingDelay !== null) {
        scheduleDeferredWatchedRemoval(video.videoId, remainingDelay);
      }
      if (
        remainingWatchlistDelay !== null &&
        watchlistController.hasAutoRemoveWatchlistItem(video.videoId)
      ) {
        watchlistController.scheduleDeferredWatchlistRemoval(
          video.videoId,
          remainingWatchlistDelay,
        );
      }
    }
  }

  function applyDownload(download: YoutubeDownload) {
    feed.videos = feed.videos.map((video) =>
      video.videoId === download.videoId ? { ...video, download } : video,
    );
    watchlistController.updateWatchlistVideo(download.videoId, (video) => ({
      ...video,
      download,
    }));
  }

  async function runDownloadAction(
    videoId: string,
    action: () => Promise<YoutubeDownload | void>,
  ) {
    if (watchlistController.downloadBusyIds.has(videoId)) return;
    watchlistController.downloadBusyIds.add(videoId);
    error = null;
    try {
      const download = await action();
      if (download) applyDownload(download);
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      watchlistController.downloadBusyIds.delete(videoId);
    }
  }

  function startDownload(video: YoutubeVideo, signedIn = false) {
    void runDownloadAction(video.videoId, () =>
      api.startYoutubeDownload(video.videoId, signedIn),
    );
  }

  function cancelDownload(video: YoutubeVideo) {
    void runDownloadAction(video.videoId, () =>
      api.cancelYoutubeDownload(video.videoId),
    );
  }

  function setDownloadPinned(video: YoutubeVideo, pinned: boolean) {
    void runDownloadAction(video.videoId, () =>
      api.setYoutubeDownloadPinned(video.videoId, pinned),
    );
  }

  function deleteDownload(video: YoutubeVideo) {
    void runDownloadAction(video.videoId, () =>
      api.deleteYoutubeDownload(video.videoId),
    );
  }

  async function copy(video: YoutubeVideo) {
    try {
      await navigator.clipboard.writeText(youtubeUrl(video.videoId));
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function open(video: YoutubeVideo) {
    try {
      await api.openExternal(youtubeUrl(video.videoId));
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  function clearFilters() {
    filters.searchText = '';
    search = '';
    filters.watchStates = [...allWatchStates];
    filters.showShorts = false;
    filters.showLive = true;
    filters.showLiveReplays = false;
    filters.showUpcoming = false;
    filters.channelId = '';
    filters.durationFilter = 'any';
    filters.publishedFilter = 'any';
    filters.downloadFilter = 'all';
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  {#if account}
    <YoutubeFeedToolbar
      bind:filters
      bind:watchlistSidebarOpen
      {refreshing}
      channels={feed.channels}
      counts={feed.counts}
      {syncStatus}
      {directVideoId}
      {directLaunchState}
      {directWatchlistBusy}
      {directInWatchlist}
      directWatchlistName={defaultWatchlist?.name ?? null}
      {addDirectVideoToWatchlist}
      {hasActiveFilters}
      {submitSearch}
      {clearFilters}
      {setGrouping}
      {refresh}
    />
  {/if}

  <div
    use:scaleYoutubeMediaScope={{
      columns: cardColumns,
      sidebarOpen: watchlistSidebarOpen,
    }}
    class="flex min-h-0 min-w-0 flex-1"
  >
    <div
      bind:this={feedScroll}
      use:connectLayoutMotion
      onwheel={handleCardGridWheel}
      data-sidebar-resize="x"
      class="relative min-h-0 min-w-0 flex-1 [scrollbar-color:var(--line-strong)_transparent] scrollbar-gutter-stable overflow-x-hidden overflow-y-auto"
      class:pr-3={!account}
      data-feed-scroll
    >
      {#if !account}
        {#if authState.status === 'pending'}
          <section
            class="panel grid min-h-72 place-items-center border border-(--line) bg-(--surface) p-8 text-center"
          >
            <div class="max-w-md">
              <p class="eyebrow">AUTH / SYSTEM BROWSER</p>
              <h2 class="mt-2 text-xl">Finish Google sign-in</h2>
              <p class="mt-3 text-sm leading-6 text-(--muted)">
                {authState.message ??
                  'Use the browser window opened by Thelxinoe. Continue signing in to your server in that browser.'}
              </p>
              <div class="mt-5">
                <Button variant="ghost" onclick={cancelAuth}
                  >Cancel sign-in</Button
                >
              </div>
            </div>
          </section>
        {:else if !['idle', 'success'].includes(authState.status)}
          <EmptyState
            eyebrow="AUTH / YOUTUBE"
            title={authFailureTitle}
            message={authState.message ?? 'The account was not connected.'}
            actionLabel="Try again"
            onAction={connect}
          />
        {:else if !clientConfigured}
          <EmptyState
            eyebrow="SETUP / GOOGLE OAUTH"
            title="Google OAuth is not configured"
            message="Add the Google Web application credentials and public server URL in Settings before YouTube can connect."
            actionLabel="Open Settings"
            onAction={() => onNavigateSettings('youtube')}
          />
        {:else}
          <EmptyState
            eyebrow="ACCOUNT / YOUTUBE"
            title="Connect a Google account"
            message="Thelxinoe requests read-only YouTube access and stores encrypted tokens on your server."
            actionLabel="Connect YouTube"
            onAction={connect}
          />
        {/if}
      {:else if feed.loading && feed.videos.length === 0}
        <EmptyState
          eyebrow="CACHE / READ"
          title="Loading your subscription feed"
          message="Thelxinoe is reading cached videos while the backend refresh continues."
        />
      {:else if feed.videos.length === 0}
        {#if syncStatus.error}
          <EmptyState
            eyebrow="SYNC / YOUTUBE"
            title={syncStatus.quotaPausedUntil
              ? 'YouTube quota exceeded'
              : 'YouTube API unavailable'}
            message={`${syncStatus.error.message}${syncStatus.quotaPausedUntil ? ` Requests resume ${quotaReset}.` : ''}`}
            actionLabel="Retry"
            onAction={() => refresh('normal')}
          />
        {:else if !filters.showShorts && feed.counts.shorts > 0 && feed.counts.all === 0}
          <EmptyState
            eyebrow="FILTER / SHORTS HIDDEN"
            title="Only Shorts are available"
            message="Shorts stay out of the feed unless you turn on the Shorts toggle."
            actionLabel="Show Shorts"
            onAction={() => (filters.showShorts = true)}
          />
        {:else if !filters.showLive && feed.counts.live > 0 && feed.counts.all === 0}
          <EmptyState
            eyebrow="FILTER / LIVE HIDDEN"
            title="Only live streams are available"
            message="Active live streams are hidden until you turn on the Live toggle."
            actionLabel="Show live streams"
            onAction={() => (filters.showLive = true)}
          />
        {:else if !filters.showLiveReplays && feed.counts.liveReplays > 0 && feed.counts.all === 0}
          <EmptyState
            eyebrow="FILTER / REPLAYS HIDDEN"
            title="Only live replays are available"
            message="Live stream replays stay out of the feed unless you turn on the Replays toggle."
            actionLabel="Show replays"
            onAction={() => (filters.showLiveReplays = true)}
          />
        {:else if !filters.showUpcoming && feed.counts.upcoming > 0 && feed.counts.all === 0}
          <EmptyState
            eyebrow="FILTER / UPCOMING HIDDEN"
            title="Only upcoming videos are available"
            message="Upcoming videos stay out of the feed unless you turn on the Upcoming toggle."
            actionLabel="Show upcoming"
            onAction={() => (filters.showUpcoming = true)}
          />
        {:else if hasActiveFilters && feed.counts.all > 0}
          <EmptyState
            eyebrow="FILTER / EMPTY"
            title="No matching videos"
            message="No subscription videos match this search and filter set."
            actionLabel="Clear filters"
            onAction={clearFilters}
          />
        {:else if feed.counts.subscribedChannelCount === 0}
          <EmptyState
            eyebrow="SUBSCRIPTIONS / EMPTY"
            title="No subscriptions available"
            message="The connected YouTube account did not return any subscribed channels."
            actionLabel="Refresh subscriptions"
            onAction={() => refresh('full')}
          />
        {:else}
          <EmptyState
            eyebrow="FEED / EMPTY"
            title="No videos fetched yet"
            message="Subscribed channels are known, but no upload metadata has been cached yet."
            actionLabel="Refresh now"
            onAction={() => refresh('normal')}
          />
        {/if}
      {:else}
        {#if feed.firstPage > 0}
          <div class="sticky top-0 z-20 flex justify-center pb-2">
            <Button size="sm" variant="secondary" onclick={loadNewestVideos}>
              Back to newest videos
            </Button>
          </div>
        {/if}
        <VideoGrid
          actions={videoActions}
          groups={groupedVideos}
          showGroupHeaders={filters.grouping !== 'none'}
          {cardColumns}
          {fadeWatchedCards}
          {youtubeCardShortcuts}
          {signedInPlaybackAvailable}
          mpvReady={playback.mpv.detected}
          ytdlpReady={playback.ytdlp.detected}
          ffmpegReady={playback.ffmpeg.detected}
          {activeVideoIds}
          {launchingVideoIds}
          downloadBusyIds={watchlistController.downloadBusyIds}
          {downloadProgress}
          watchlists={watchlistController.watchlists}
          hasMore={feed.hasMore}
          loadingMore={feed.loadingMore}
          onLoadMore={() => feed.loadVideos(false)}
        />
      {/if}
    </div>
    {#if account}
      <div
        data-youtube-watchlist-frame
        class={`relative z-20 min-h-0 shrink-0 overflow-visible ${watchlistSidebarOpen ? '' : 'pointer-events-none'}`}
        aria-hidden={!watchlistSidebarOpen}
      >
        <div
          data-youtube-watchlist-scale
          data-sidebar-resize="x"
          data-sidebar-resize-zoom
          class="absolute top-0 right-0"
        >
          <WatchlistSidebar
            actions={videoActions}
            watchlists={watchlistController.watchlists}
            selectedWatchlistId={watchlistController.selectedWatchlistId}
            {downloadProgress}
            downloadBusyIds={watchlistController.downloadBusyIds}
            {activeVideoIds}
            {launchingVideoIds}
            mpvReady={playback.mpv.detected}
            ytdlpReady={playback.ytdlp.detected}
            ffmpegReady={playback.ffmpeg.detected}
            onSelect={(watchlistId) =>
              (watchlistController.selectedWatchlistId = watchlistId)}
            onCreate={watchlistController.createWatchlist}
            onUpdate={watchlistController.updateWatchlist}
            onDelete={watchlistController.deleteWatchlist}
            onReorder={watchlistController.reorderWatchlist}
            onRemove={(watchlistId, video) =>
              void watchlistController.removeFromWatchlist(watchlistId, video)}
          />
        </div>
      </div>
    {/if}
  </div>
</div>

{#if actionBusy}<div
    class="pointer-events-none fixed right-5 bottom-5 z-40 border border-(--line-strong) bg-(--surface-strong) px-3 py-2 text-xs text-(--accent)"
  >
    Updating progress…
  </div>{/if}
