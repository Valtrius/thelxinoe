<script lang="ts">
  import { providerPages } from '../pages';
  import { onMount, untrack } from 'svelte';
  import { toolsApi } from './tools-api';
  import { api as request, desktop } from '../api';
  import { captureSession } from '../session';
  import { LatestRequest } from '../latest-request';
  import { appearance, updateAppearance } from '../appearance';
  import type { MediaChoice } from '../playback';
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
  import ToastViewport from './components/ui/ToastViewport.svelte';

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
    playing: MediaChoice | null;
    play: (choice: MediaChoice) => Promise<void>;
    settings: (section: string) => void;
    watchlistId?: number;
    watchlistChanged: (id: number) => void;
  }>();
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
  let pendingGoogle = $state(false);
  let nativeReady = $state(true);
  let disposed = false;
  let loadingPlatform: typeof platform | null = null;
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
  const authState = $derived<AuthState>({
    platform: platform === 'youtube' ? 'youtube' : 'twitch',
    operationId: platform,
    status:
      rawAccount?.pending ||
      (platform === 'youtube' && pendingGoogle && !account)
        ? 'pending'
        : rawAccount?.account.status === 'reconnect_required'
          ? 'error'
          : 'idle',
    message:
      rawAccount?.pending?.error ||
      (rawAccount?.account.status === 'reconnect_required'
        ? 'Reconnect your account to resume synchronization.'
        : undefined),
    userCode: rawAccount?.pending?.user_code,
    verificationUri: rawAccount?.pending?.verification_uri,
    expiresAt: iso(rawAccount?.pending?.expires_at),
  });
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
    activeSessionCount: playing ? 1 : 0,
  });
  const activeSessions = $derived<PlaybackSession[]>(
    playing && playing.id.startsWith(platform + ':')
      ? [
          {
            sessionId: playing.id,
            platform,
            mediaId: activeMediaId(playing),
            ipcEndpoint: '',
            startedAt: '',
            playbackState: 'playing',
            connected: true,
            sourceKind: 'remote',
          },
        ]
      : [],
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
      if (
        selected === 'youtube' &&
        (value as OnlineAccount).account.status === 'connected'
      )
        pendingGoogle = false;
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
      if (current()) loadingPlatform = null;
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
      pendingGoogle = false;
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
    const timer = setInterval(refreshVisible, 5000);
    document.addEventListener('visibilitychange', refreshVisible);
    return () => {
      disposed = true;
      accountRequests.invalidate();
      clearInterval(timer);
      document.removeEventListener('visibilitychange', refreshVisible);
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
    pendingGoogle = platform === 'youtube';
    void load();
  }
</script>

<div
  class="provider-surface min-w-0 text-[1rem]"
  data-sidebar-resize="xy"
  data-sidebar-resize-origin
>
  {#if platform === 'youtube'}
    {#await providerPages.YoutubeView()}<p role="status">
        Loading provider…
      </p>{:then { default: YoutubeView }}<YoutubeView
        {admin}
        {account}
        {watchlistController}
        onWatchlistSelected={watchlistChanged}
        showWatchlist={watchlistId !== undefined}
        {syncStatus}
        {playback}
        {activeSessions}
        cardColumns={$appearance.card_columns}
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
        onCardColumnsChange={(card_columns) =>
          updateAppearance({ card_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {:else if platform === 'twitch'}
    {#await providerPages.TwitchView()}<p role="status">
        Loading provider…
      </p>{:then { default: TwitchView }}<TwitchView
        {admin}
        {account}
        {syncStatus}
        {playback}
        {activeSessions}
        cardColumns={$appearance.card_columns}
        {authState}
        clientConfigured={!!rawAccount?.configured}
        {dataRevision}
        onAuthStarted={authStarted}
        onNavigateSettings={navigateSettings}
        onAccountChanged={() => void load()}
        onCardColumnsChange={(card_columns) =>
          updateAppearance({ card_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {:else}
    {#await providerPages.KickView()}<p role="status">
        Loading provider…
      </p>{:then { default: KickView }}<KickView
        {admin}
        connected={!!kick?.connected}
        onAccountChanged={() => void load()}
        {syncStatus}
        {playback}
        {activeSessions}
        cardColumns={$appearance.card_columns}
        metadataConfigured={!!kick?.configured}
        {dataRevision}
        onNavigateSettings={navigateSettings}
        onCardColumnsChange={(card_columns) =>
          updateAppearance({ card_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {/if}
  <ToastViewport />
</div>
