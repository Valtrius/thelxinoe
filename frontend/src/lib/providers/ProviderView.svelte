<script lang="ts">
  import { onServerEvent, revisions } from '../server-events.svelte';
  import { providerPages } from '../pages';
  import { onMount, untrack } from 'svelte';
  import { toolsApi } from './tools-api';
  import { api as request, desktop } from '../api';
  import { captureSession } from '../session';
  import { LatestRequest } from '../latest-request';
  import { RefreshTask } from '../refresh-task';
  import { navigationCache } from '../navigation-cache';
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
    playing,
    play,
    settings,
    watchlistId,
    watchlistChanged,
  } = $props<{
    platform: 'youtube' | 'twitch' | 'kick';
    admin: boolean;
    userId: string;
    playing: PlayingMedia[];
    play: (choice: MediaChoice) => Promise<void>;
    settings: (section: string) => void;
    watchlistId?: number;
    watchlistChanged: (id: number) => void;
  }>();
  const cardColumns = $derived(
    compactViewport.current ? 2 : $appearance.video_columns,
  );
  const disconnect = untrack(() => connectPresentation(play));
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
  const loadedAccounts = $state<Partial<Record<typeof platform, boolean>>>({});
  const accountRequests = new LatestRequest();
  let lastSnapshot = '';
  let forceData = false;
  let reads: RefreshTask | undefined;
  const load = () => reads?.refresh() ?? Promise.resolve();
  let refreshAfter = 0;
  let refreshTimer: ReturnType<typeof setTimeout>;
  function refreshData() {
    if (disposed || !ownsSession()) return;
    if (document.hidden) {
      forceData = true;
      void load();
      return;
    }
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
  // This platform's media, by the platform's own id.
  const activeMedia = $derived<PlayingMedia[]>(
    playing
      .filter((media: PlayingMedia) => media.id.startsWith(platform + ':'))
      .map((media: PlayingMedia) => ({
        id: activeMediaId(media),
        launching: media.launching,
      })),
  );
  async function readAccount() {
    if (disposed || !ownsSession()) return;
    const selected: 'youtube' | 'twitch' | 'kick' = platform;
    const current = accountRequests.begin();
    try {
      const path = `/online/${selected}`;
      await navigationCache.read(
        selected,
        path,
        () => request<OnlineAccount | KickFeed>(path),
        (value) => {
          if (disposed || !current() || selected !== platform) return;
          loadedAccounts[selected] = true;
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
          if (snapshot !== lastSnapshot || forceData) {
            if (lastSnapshot && snapshot !== lastSnapshot)
              navigationCache.invalidate(selected);
            lastSnapshot = snapshot;
            forceData = false;
            refreshData();
          }
        },
        0,
      );
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
      }
    }
  }
  $effect(() => {
    void platform;
    const task = new RefreshTask(readAccount, () =>
      authPending ? 1000 : 5000,
    );
    reads = task;
    untrack(() => {
      forceData = true;
      task.start();
    });
    return () => {
      task.stop();
      accountRequests.invalidate();
      clearTimeout(refreshTimer);
    };
  });
  let revisionReady = false;
  $effect(() => {
    void revisions.media;
    untrack(() => {
      if (!revisionReady) {
        revisionReady = true;
        return;
      }
      // Library and YouTube events change the YouTube feed. Twitch and Kick
      // feeds change with their account's sync, which the read compares.
      if (platform === 'youtube') forceData = true;
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
    const stopDownloads = onServerEvent((event) => {
      if (event.kind === 'online.download.progress')
        youtubeDownload = downloadEvent(
          event.payload as Parameters<typeof downloadEvent>[0],
        );
    });
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
    return () => {
      disposed = true;
      accountRequests.invalidate();
      clearTimeout(refreshTimer);
      clearToasts();
      stopDownloads();
      window.removeEventListener(
        'thelxinoe-provider-auth-cancelled',
        cancelled,
      );
      window.removeEventListener('thelxinoe-youtube-progress', progress);
      window.removeEventListener('thelxinoe-youtube-download-removed', removed);
      watchlistController.reset();
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
        playerReady={nativeReady}
        downloadsEnabled={accounts.youtube?.downloads_enabled ?? false}
        {activeMedia}
        {cardColumns}
        fadeWatchedCards={$appearance.fade_watched}
        youtubeCardShortcuts={$appearance.youtube_card_shortcuts}
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
        playerReady={nativeReady}
        {activeMedia}
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
        onAccountChanged={load}
        {syncStatus}
        playerReady={nativeReady}
        {activeMedia}
        {cardColumns}
        metadataConfigured={!!kick?.configured}
        feed={kick}
        onNavigateSettings={navigateSettings}
        onCardColumnsChange={(video_columns) =>
          updateAppearance({ video_columns })}
      />{:catch error}<Notice role="alert" variant="error"
        >{String(error)}</Notice
      >{/await}
  {/if}
</div>
