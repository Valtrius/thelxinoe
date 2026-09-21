<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { toolsApi } from './tools-api';
  import { api as request, desktop } from '../api';
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
  import YoutubeView from './components/youtube/YoutubeView.svelte';
  import TwitchView from './components/twitch/TwitchView.svelte';
  import KickView from './components/kick/KickView.svelte';
  import ToastViewport from './components/ui/ToastViewport.svelte';

  let { platform, userId, revision, playing, play, settings } = $props<{
    platform: 'youtube' | 'twitch' | 'kick';
    userId: string;
    revision: number;
    playing: MediaChoice | null;
    play: (choice: MediaChoice) => Promise<void>;
    settings: (section: string) => void;
  }>();
  const disconnect = untrack(() => connectPresentation(userId, play));
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
  let loading = false;
  let lastSnapshot = '';
  let refreshAfter = 0;
  let refreshTimer: ReturnType<typeof setTimeout>;
  function refreshData() {
    if (disposed) return;
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
    if (loading || disposed) return;
    loading = true;
    try {
      const [youtube, twitch, tracked] = await Promise.all([
        request<OnlineAccount>('/online/youtube'),
        request<OnlineAccount>('/online/twitch'),
        request<KickFeed>('/online/kick'),
      ]);
      if (disposed) return;
      accounts = { youtube, twitch };
      kick = tracked;
      if (youtube.account.status === 'connected') pendingGoogle = false;
      const snapshot = JSON.stringify([youtube, twitch, tracked]);
      if (snapshot !== lastSnapshot) {
        lastSnapshot = snapshot;
        refreshData();
      }
    } catch (error) {
      if (!disposed)
        showToast({
          key: 'providers-load',
          tone: 'error',
          title: 'Could not refresh accounts',
          message: normalizeError(error).message,
        });
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void revision;
    untrack(() => {
      refreshData();
      void load();
    });
  });
  $effect(() => {
    const selected = $preferences['youtube-selected-watchlist-id'];
    if (selected && Number.isSafeInteger(Number(selected)))
      watchlistController.selectedWatchlistId = Number(selected);
  });
  $effect(() => {
    const selected = watchlistController.selectedWatchlistId;
    if (selected !== null)
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
    const timer = setInterval(() => void load(), 5000);
    return () => {
      disposed = true;
      clearInterval(timer);
      clearTimeout(refreshTimer);
      clearToasts();
      window.removeEventListener('thelxinoe-download-progress', downloading);
      window.removeEventListener(
        'thelxinoe-provider-auth-cancelled',
        cancelled,
      );
      window.removeEventListener('thelxinoe-youtube-progress', progress);
      window.removeEventListener('thelxinoe-youtube-download-removed', removed);
      void watchlistController
        .flushWatchlistRemovals()
        .finally(() => watchlistController.reset());
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
    <YoutubeView
      {account}
      {watchlistController}
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
      onCardColumnsChange={(card_columns) => updateAppearance({ card_columns })}
    />
  {:else if platform === 'twitch'}
    <TwitchView
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
      onCardColumnsChange={(card_columns) => updateAppearance({ card_columns })}
    />
  {:else}
    <KickView
      {syncStatus}
      {playback}
      {activeSessions}
      cardColumns={$appearance.card_columns}
      metadataConfigured={!!kick?.configured}
      {dataRevision}
      onNavigateSettings={navigateSettings}
      onCardColumnsChange={(card_columns) => updateAppearance({ card_columns })}
    />
  {/if}
  <ToastViewport />
</div>
