<script lang="ts">
  import { providerPages } from '../pages';
  import { onMount, untrack } from 'svelte';
  import { toolsApi } from './tools-api';
  import { api as request, desktop } from '../api';
  import { captureSession } from '../session';
  import { LatestRequest } from '../latest-request';
  import { appearance, updateAppearance } from '../appearance';
  import { compactViewport } from '../responsive';
  import type { MediaChoice, PlayingMedia } from '../playback';
  import {
    connectPresentation,
    activeMediaId,
    iso,
    normalizeError,
    downloadEvent,
    preferences,
    type OnlineAccount,
    type KickFeed,
  } from './api';
  import type {
    AuthState,
    PlatformAccount,
    PlaybackDiagnostics,
    PlaybackSession,
    SyncStatus,
    YoutubeProgressEvent,
    YoutubeProgressUpdate,
    YoutubeDownloadEvent,
  } from './types';
  import { createYoutubeWatchlists } from './youtube-watchlist-controller.svelte';
  import { showToast, clearToasts } from './toasts';
  import Notice from '../ui/Notice.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';

  let {
    platform,
    admin,
    userId,
    revision,
    playing,
    play,
    settings,
    watchlistId,
    watchlistChanged,
  } = $props<{
    platform: 'youtube' | 'twitch' | 'kick';
    admin: boolean;
    userId: string;
    revision: number;
    playing: PlayingMedia[];
    play: (choice: MediaChoice) => Promise<void>;
    settings: (section: string) => void;
    watchlistId?: number;
    watchlistChanged: (id: number) => void;
  }>();
  const cardColumns = $derived(
    compactViewport.current ? 2 : $appearance.video_columns,
  );
  const disconnect = untrack(() => connectPresentation(userId, play));
  const ownsSession = captureSession();
  const watchlistController = createYoutubeWatchlists(
    Number(preferences.getItem('youtube-selected-watchlist-id')),
  );
  let accounts = $state<Partial<Record<'youtube' | 'twitch', OnlineAccount>>>(
    {},
  );
  let kick = $state<KickFeed | null>(null);
  let dataRevision = $state(0);
  let youtubeProgress = $state<YoutubeProgressUpdate | null>(null);
  let youtubeDownload = $state<YoutubeDownloadEvent | null>(null);
  let youtubeDownloadRemoval = $state<{
    videoId: string;
    sequence: number;
  } | null>(null);
  let pendingGoogleUntil = $state<number | null>(null);
  let authNow = $state(Date.now());
  let nativeReady = $state(true);
  let disposed = false;
  let loadingPlatform: typeof platform | null = null;
  const loadedAccounts = $state<Partial<Record<typeof platform, boolean>>>({});
  const accountRequests = new LatestRequest();
  let lastSnapshot = '';
  let refreshAfter = 0;
  let refreshTimer: ReturnType<typeof setTimeout>;
  function refreshData() {
    if (disposed || !ownsSession()) return;
    if (platform !== 'youtube') {
      dataRevision++;
      return;
    }
    if (watchlistController.pendingManualWatchedVideoIds.size)
      refreshAfter = Date.now() + 5000;
    clearTimeout(refreshTimer);
    if (Date.now() < refreshAfter) {
      refreshTimer = setTimeout(refreshData, refreshAfter - Date.now() + 20);
      return;
    }
    dataRevision++;
    void watchlistController.loadWatchlists();
  }
  const rawAccount = $derived(
    platform === 'kick' ? null : accounts[platform as 'youtube' | 'twitch'],
  );
  const account = $derived<PlatformAccount | null>(
    rawAccount?.account.status === 'connected'
      ? {
          platform: platform === 'youtube' ? 'youtube' : 'twitch',
          externalUserId: rawAccount.account.external_id || userId,
          displayName: rawAccount.account.display_name,
          connectedAt: iso(rawAccount.account.updated_at) ?? '',
          updatedAt: iso(rawAccount.account.updated_at) ?? '',
        }
      : null,
  );
  const authState = $derived.by((): AuthState => {
    const pending = rawAccount?.pending;
    const expires = pending
      ? pending.expires_at * 1000
      : platform === 'youtube'
        ? pendingGoogleUntil
        : null;
    const expired = expires !== null && expires <= authNow;
    return {
      platform: platform === 'youtube' ? 'youtube' : 'twitch',
      operationId: platform,
      phase: pending?.phase === 'connecting' ? 'connecting' : 'waiting',
      status:
        pending?.phase === 'failed'
          ? 'error'
          : expired
            ? 'expired'
            : pending || (expires !== null && !account)
              ? 'pending'
              : rawAccount?.account.status === 'reconnect_required'
                ? 'error'
                : 'idle',
      message: expired
        ? 'The sign-in request expired. Start again.'
        : pending?.error ||
          (!pending &&
          expires === null &&
          rawAccount?.account.status === 'reconnect_required'
            ? 'Reconnect your account to resume synchronization.'
            : undefined),
      userCode: pending?.user_code,
      verificationUri: pending?.verification_uri,
      expiresAt: expires === null ? null : new Date(expires).toISOString(),
    };
  });
  const authPending = $derived(authState.status === 'pending');
  $effect(() => {
    const timer = setInterval(
      () => {
        authNow = Date.now();
        if (!document.hidden) void load();
      },
      authPending ? 1000 : 5000,
    );
    return () => clearInterval(timer);
  });
  // YouTube's daily quota resets at midnight Pacific time after the server's quota day.
  function quotaReset(day: string): string | null {
    const [year, month, date] = day.split('-').map(Number);
    if (!year || !month || !date) return null;
    // Pacific midnight is 07:00 UTC in summer and 08:00 UTC in winter.
    for (const hour of [7, 8]) {
      const at = new Date(Date.UTC(year, month - 1, date + 1, hour));
      const pacific = new Intl.DateTimeFormat('en-US', {
        timeZone: 'America/Los_Angeles',
        hour: 'numeric',
        hourCycle: 'h23',
      }).format(at);
      if (Number(pacific) === 0) return at.toISOString();
    }
    return null;
  }
  const syncStatus = $derived.by((): SyncStatus => {
    const sync = rawAccount?.sync;
    const kickError = kick?.items.find((c) => c.error)?.error;
    const last =
      platform === 'kick'
        ? kick?.items.length
          ? Math.min(...kick.items.map((c) => c.updated_at))
          : 0
        : sync?.last_complete;
    const error = platform === 'kick' ? kickError : sync?.error;
    return {
      platform,
      phase: sync?.phase ?? (sync?.in_progress ? 'Refreshing' : 'Idle'),
      isRefreshing: (sync?.in_progress ?? false) && !rawAccount?.quota?.blocked,
      completed: 0,
      lastSuccessAt: iso(last),
      stale: !!error || !last,
      quotaPausedUntil:
        platform === 'youtube' && rawAccount?.quota?.blocked
          ? quotaReset(rawAccount.quota.day)
          : null,
      error: error ? normalizeError(error) : null,
    };
  });
  // Extraction tools are installed and checked by the server on demand. These
  // readiness inputs describe whether an operation can be requested by this UI.
  const playback = $derived<PlaybackDiagnostics>({
    mpv: { kind: desktop ? 'mpv' : 'browser', detected: nativeReady },
    ytdlp: { kind: 'server', detected: true },
    streamlink: { kind: 'server', detected: true },
    ffmpeg: {
      kind: 'server-downloads',
      detected: accounts.youtube?.downloads_enabled ?? false,
    },
    activeSessionCount: playing.length,
  });
  const activeSessions = $derived<PlaybackSession[]>(
    playing
      .filter((media: PlayingMedia) => media.id.startsWith(platform + ':'))
      .map((media: PlayingMedia) => ({
        sessionId: media.id,
        platform,
        mediaId: activeMediaId(media),
        ipcEndpoint: '',
        startedAt: '',
        playbackState: media.launching ? 'starting' : 'playing',
        connected: true,
        sourceKind: 'remote',
      })),
  );
  async function load() {
    if (loadingPlatform === platform || disposed || !ownsSession()) return;
    const selected: 'youtube' | 'twitch' | 'kick' = platform;
    loadingPlatform = selected;
    const current = accountRequests.begin();
    try {
      const value = await request<OnlineAccount | KickFeed>(
        `/online/${selected}`,
      );
      if (disposed || !current() || selected !== platform) return;
      if (selected === 'kick') kick = value as KickFeed;
      else accounts[selected] = value as OnlineAccount;
      authNow = Date.now();
      if (
        selected === 'youtube' &&
        ((value as OnlineAccount).pending ||
          (value as OnlineAccount).account.status === 'connected')
      )
        pendingGoogleUntil = null;
      const snapshot = JSON.stringify([selected, value]);
      if (snapshot !== lastSnapshot) {
        lastSnapshot = snapshot;
        refreshData();
      }
    } catch (error) {
      if (!disposed && current() && selected === platform)
        showToast({
          key: 'providers-load',
          tone: 'error',
          title: 'Could not refresh accounts',
          message: normalizeError(error).message,
        });
    } finally {
      if (current()) {
        loadedAccounts[selected] = true;
        loadingPlatform = null;
      }
    }
  }
  $effect(() => {
    void platform;
    void revision;
    untrack(() => {
      refreshData();
      void load();
    });
  });
  $effect(() => {
    const selected = $preferences['youtube-selected-watchlist-id'];
    const linked = watchlistId;
    if (
      platform === 'youtube' &&
      (linked || selected) &&
      Number.isSafeInteger(Number(linked ?? selected))
    )
      watchlistController.selectedWatchlistId = Number(linked ?? selected);
  });
  $effect(() => {
    const selected = watchlistController.selectedWatchlistId;
    if (platform === 'youtube' && selected !== null)
      preferences.setItem('youtube-selected-watchlist-id', String(selected));
  });
  onMount(() => {
    const downloading = (event: Event) => {
      youtubeDownload = downloadEvent(
        (event as CustomEvent<Parameters<typeof downloadEvent>[0]>).detail,
      );
    };
    window.addEventListener('thelxinoe-download-progress', downloading);
    const progress = (event: Event) => {
      const detail = (event as CustomEvent<YoutubeProgressEvent>).detail;
      if (watchlistController.pendingManualWatchedVideoIds.has(detail.videoId))
        refreshAfter = Date.now() + 5000;
      youtubeProgress = {
        ...detail,
        manualWatchPending: watchlistController.applyProgress(detail),
      };
    };
    const removed = (event: Event) => {
      youtubeDownloadRemoval = {
        videoId: (event as CustomEvent<string>).detail,
        sequence: Date.now(),
      };
    };
    const cancelled = () => {
      pendingGoogleUntil = null;
      void load();
    };
    window.addEventListener('thelxinoe-provider-auth-cancelled', cancelled);
    window.addEventListener('thelxinoe-youtube-progress', progress);
    window.addEventListener('thelxinoe-youtube-download-removed', removed);
    void load();
    if (desktop)
      void toolsApi
        .check('mpv')
        .then(() => toolsApi.get())
        .then((value) => {
          nativeReady = !!value.tools.find((tool) => tool.id === 'mpv')
            ?.diagnostic?.detected;
        })
        .catch(() => {
          nativeReady = false;
        });
    const refreshVisible = () => {
      if (!document.hidden) void load();
    };
    document.addEventListener('visibilitychange', refreshVisible);
    window.addEventListener('focus', refreshVisible);
    return () => {
      disposed = true;
      accountRequests.invalidate();
      document.removeEventListener('visibilitychange', refreshVisible);
      window.removeEventListener('focus', refreshVisible);
      clearTimeout(refreshTimer);
      clearToasts();
      window.removeEventListener('thelxinoe-download-progress', downloading);
      window.removeEventListener(
        'thelxinoe-provider-auth-cancelled',
        cancelled,
      );
      window.removeEventListener('thelxinoe-youtube-progress', progress);
      window.removeEventListener('thelxinoe-youtube-download-removed', removed);
      if (ownsSession())
        void watchlistController
          .flushWatchlistRemovals()
          .finally(() => watchlistController.reset());
      else watchlistController.reset();
      disconnect();
    };
  });
  function navigateSettings(section: string) {
    settings(
      section === 'tools'
        ? 'mpv'
        : section === 'kick' || !rawAccount?.configured
          ? 'providers'
          : 'online',
    );
  }
  function authStarted() {
    pendingGoogleUntil =
      platform === 'youtube' && desktop ? Date.now() + 600_000 : null;
    void load();
  }
</script>

<div
  class="provider-surface min-w-0 text-[1rem]"
  class:h-full={platform === 'youtube'}
>
  {#if !loadedAccounts[platform]}
    <MediaSkeleton
      label={`Loading ${platform === 'youtube' ? 'YouTube' : platform === 'twitch' ? 'Twitch' : 'Kick'}`}
      shape="landscape"
    />
  {:else if platform === 'youtube'}
    {#await providerPages.YoutubeView()}<MediaSkeleton
        label="Loading YouTube"
        shape="landscape"
      />{:then { default: YoutubeView }}<YoutubeView
        {admin}
        {account}
        {watchlistController}
        onWatchlistSelected={watchlistChanged}
        showWatchlist={watchlistId !== undefined}
        {syncStatus}
        {playback}
        {activeSessions}
        {cardColumns}
        fadeWatchedCards={$appearance.fade_watched}
        youtubeCardShortcuts={$appearance.youtube_card_shortcuts}
        signedInPlaybackAvailable={false}
        {authState}
        clientConfigured={!!rawAccount?.configured &&
          !!rawAccount.linking_available}
        {dataRevision}
        {youtubeProgress}
        {youtubeDownload}
        {youtubeDownloadRemoval}
        onAuthStarted={authStarted}
        onNavigateSettings={navigateSettings}
        onAccountChanged={() => void load()}
        onYoutubeCardShortcutsChanged={(youtube_card_shortcuts) =>
          updateAppearance({ youtube_card_shortcuts })}
        onCardColumnsChange={(video_columns) =>
          updateAppearance({ video_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {:else if platform === 'twitch'}
    {#await providerPages.TwitchView()}<MediaSkeleton
        label="Loading Twitch"
        shape="landscape"
      />{:then { default: TwitchView }}<TwitchView
        {admin}
        {account}
        {syncStatus}
        {playback}
        {activeSessions}
        {cardColumns}
        {authState}
        clientConfigured={!!rawAccount?.configured}
        {dataRevision}
        onAuthStarted={authStarted}
        onNavigateSettings={navigateSettings}
        onAccountChanged={() => void load()}
        onCardColumnsChange={(video_columns) =>
          updateAppearance({ video_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {:else}
    {#await providerPages.KickView()}<MediaSkeleton
        label="Loading Kick"
        shape="landscape"
      />{:then { default: KickView }}<KickView
        {admin}
        connected={!!kick?.connected}
        onAccountChanged={() => void load()}
        {syncStatus}
        {playback}
        {activeSessions}
        {cardColumns}
        metadataConfigured={!!kick?.configured}
        {dataRevision}
        onNavigateSettings={navigateSettings}
        onCardColumnsChange={(video_columns) =>
          updateAppearance({ video_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {/if}
</div>
