<script lang="ts">
  import {
    captureScrollAnchor,
    restoreScrollAnchor,
  } from '../../scroll-anchor';
  import { LatestRequest } from '../../latest-request';
  import { tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { Copy, ExternalLink, RefreshCw, X } from '@lucide/svelte';
  import { api, normalizeError } from '../../api';
  import { createCardGridWheelHandler } from '../../card-grid-wheel';
  import { scaleCardScope } from '../../card-grid-zoom';
  import { createLayoutMotion } from '../../layout-motion';
  import { dismissToastByKey, showToast } from '../../toasts';
  import type {
    AppError,
    AuthState,
    PlatformAccount,
    PlaybackDiagnostics,
    PlaybackSession,
    SyncStatus,
    TwitchLiveStream,
  } from '../../types';
  import {
    isPlaybackLaunching,
    relativeTime,
    twitchChatUrl,
    twitchUrl,
  } from '../../utils';
  import Button from '../ui/Button.svelte';
  import EmptyState from '../ui/EmptyState.svelte';
  import StreamCard from './StreamCard.svelte';

  let {
    account,
    syncStatus,
    playback,
    activeSessions,
    cardColumns,
    authState,
    clientConfigured,
    dataRevision,
    onAuthStarted,
    onNavigateSettings,
    onAccountChanged,
    onCardColumnsChange,
  }: {
    account?: PlatformAccount | null;
    syncStatus: SyncStatus;
    playback: PlaybackDiagnostics;
    activeSessions: PlaybackSession[];
    cardColumns: number;
    authState: AuthState;
    clientConfigured: boolean;
    dataRevision: number;
    onAuthStarted: (operationId: string) => void;
    onNavigateSettings: (section: 'twitch' | 'tools') => void;
    onAccountChanged: () => void;
    onCardColumnsChange: (columns: number) => void;
  } = $props();

  let streams = $state<TwitchLiveStream[]>([]);
  let loading = $state(true);
  let error = $state<AppError | null>(null);
  let lastSyncErrorFingerprint = $state<string | null>(null);
  let observedDataRevision = $state<number | null>(null);
  let feedScroll = $state<HTMLElement | null>(null);
  const layoutMotion = createLayoutMotion();
  const connectLayoutMotion = layoutMotion.connect;
  const pendingChannelLogins = new SvelteSet<string>();
  const requests = new LatestRequest();
  const playbackReady = $derived(
    playback.mpv.detected && playback.streamlink.detected,
  );
  const activeChannelLogins = $derived(
    new Set(
      activeSessions
        .filter((session) => session.platform === 'twitch')
        .map((session) => session.mediaId.toLowerCase()),
    ),
  );
  const launchingChannelLogins = $derived(
    new Set([
      ...pendingChannelLogins,
      ...activeSessions
        .filter(
          (session) =>
            session.platform === 'twitch' &&
            isPlaybackLaunching(session.playbackState),
        )
        .map((session) => session.mediaId.toLowerCase()),
    ]),
  );
  const authFailureTitle = $derived.by(() => {
    switch (authState.status) {
      case 'cancelled':
        return 'Twitch sign-in cancelled';
      case 'denied':
        return 'Twitch access denied';
      case 'expired':
        return 'Twitch device code expired';
      default:
        return 'Twitch sign-in failed';
    }
  });
  const snapshotKey = $derived(account?.externalUserId ?? 'disconnected');

  $effect(() => {
    const requestKey = snapshotKey;
    untrack(() => void load(requestKey));
  });

  $effect(() => {
    const revision = dataRevision;
    if (observedDataRevision === null) {
      observedDataRevision = revision;
      return;
    }
    if (revision === observedDataRevision) return;
    observedDataRevision = revision;
    const requestKey = snapshotKey;
    untrack(() => void load(requestKey, true));
  });

  $effect(() => {
    if (account && !playbackReady) {
      showToast({
        key: 'twitch-playback-missing',
        tone: 'warning',
        title: 'Playback dependencies missing',
        message: 'Configure the Windows player in Settings to watch channels.',
        actionLabel: 'Player settings',
        onAction: () => onNavigateSettings('tools'),
        durationMs: null,
      });
    } else {
      dismissToastByKey('twitch-playback-missing');
    }
  });

  $effect(() => {
    if (error) {
      showToast({
        key: 'twitch-action-error',
        tone: 'error',
        title: 'Twitch action failed',
        message: error.message,
        detail: error.technical,
        actionLabel: 'Retry snapshot',
        onAction: load,
        durationMs: null,
      });
    } else {
      dismissToastByKey('twitch-action-error');
    }
  });

  $effect(() => {
    const syncError = syncStatus.error;
    if (!syncError) {
      lastSyncErrorFingerprint = null;
      dismissToastByKey('twitch-sync-error');
      return;
    }
    const fingerprint = `${syncError.code}:${syncError.message}:${syncError.technical}`;
    if (fingerprint === lastSyncErrorFingerprint) return;
    lastSyncErrorFingerprint = fingerprint;
    showToast({
      key: 'twitch-sync-error',
      tone: syncStatus.rateLimitedUntil ? 'warning' : 'error',
      title: syncStatus.rateLimitedUntil
        ? 'Twitch rate limit reached'
        : 'Twitch refresh failed',
      message: `${syncError.message}${streams.length > 0 ? ` Cached channels remain available. Last good sync ${relativeTime(syncStatus.lastSuccessAt)}.` : ''}`,
      detail: syncError.technical,
      actionLabel: syncStatus.rateLimitedUntil ? null : 'Retry',
      onAction: syncStatus.rateLimitedUntil ? null : refresh,
      durationMs: syncStatus.rateLimitedUntil ? null : 10_000,
    });
  });

  async function load(requestKey = snapshotKey, preserveScroll = false) {
    const current = requests.begin();
    if (!account) {
      streams = [];
      loading = false;
      return;
    }
    const anchor = preserveScroll
      ? captureScrollAnchor(feedScroll, 'data-stream-id')
      : null;
    loading = streams.length === 0;
    error = null;
    try {
      const next = await api.twitchStreams();
      if (requestKey !== snapshotKey || !current()) return;
      const layoutSnapshot = layoutMotion.capture();
      streams = next;
      await tick();
      if (!current()) return;
      restoreScrollAnchor(feedScroll, 'data-stream-id', anchor);
      layoutMotion.play(layoutSnapshot);
    } catch (caught) {
      if (current()) error = normalizeError(caught);
    } finally {
      if (current()) loading = false;
    }
  }

  async function connect() {
    error = null;
    try {
      onAuthStarted(await api.startTwitchAuth());
    } catch (caught) {
      error = normalizeError(caught);
      if (error.category === 'authentication') onAccountChanged();
    }
  }

  async function cancelAuth() {
    if (!authState.operationId) return;
    try {
      await api.cancelTwitchAuth(authState.operationId);
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function refresh() {
    error = null;
    try {
      await api.syncTwitch();
    } catch (caught) {
      error = normalizeError(caught);
      if (error.category === 'authentication') onAccountChanged();
    }
  }

  async function watch(stream: TwitchLiveStream) {
    const login = stream.login.toLowerCase();
    if (pendingChannelLogins.has(login) || activeChannelLogins.has(login))
      return;
    pendingChannelLogins.add(login);
    try {
      await api.launchTwitch(stream.login);
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      pendingChannelLogins.delete(login);
    }
  }

  async function copy(stream: TwitchLiveStream) {
    try {
      await navigator.clipboard.writeText(twitchUrl(stream.login));
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function open(url: string) {
    try {
      await api.openExternal(url);
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  const handleCardGridWheel = createCardGridWheelHandler({
    anchorAttribute: 'data-stream-id',
    motion: layoutMotion,
    getColumns: () => cardColumns,
    setColumns: (columns) => onCardColumnsChange(columns),
  });
</script>

<div class="flex h-full min-h-0 flex-col">
  {#if account}
    <div
      class="mr-3 flex shrink-0 flex-wrap items-center gap-2 border-b border-(--line) pb-2"
    >
      <span class="ml-auto text-[0.65rem] text-(--muted)">
        {#if syncStatus.isRefreshing}{syncStatus.phase} · {syncStatus.completed}/{syncStatus.total ??
            '—'}{:else}Updated {relativeTime(syncStatus.lastSuccessAt)}{/if}
      </span>
      <Button
        size="sm"
        variant="secondary"
        disabled={syncStatus.isRefreshing}
        onclick={refresh}
      >
        <RefreshCw
          class={`size-3.5 ${syncStatus.isRefreshing ? 'animate-spin' : ''}`}
        />Refresh
      </Button>
    </div>
  {/if}

  <div
    bind:this={feedScroll}
    use:connectLayoutMotion
    use:scaleCardScope={cardColumns}
    onwheel={handleCardGridWheel}
    data-sidebar-resize="x"
    class={`relative min-h-0 flex-1 [scrollbar-color:var(--line-strong)_transparent] scrollbar-gutter-stable overflow-x-hidden overflow-y-auto ${account ? 'pt-3 pr-1' : 'pr-3'}`}
    data-feed-scroll
  >
    {#if !account}
      {#if authState.status === 'pending'}
        <section
          class="panel border border-(--line-strong) bg-(--surface) p-6 text-center"
        >
          <p class="eyebrow">DEVICE AUTH / TWITCH</p>
          <h2 class="mt-2 text-xl">Enter this code in Twitch</h2>
          <button
            type="button"
            class="mt-5 border border-(--line-strong) bg-(--accent-soft) px-8 py-4 font-mono text-3xl tracking-[0.22em] text-(--accent)"
            aria-label="Copy Twitch device code"
            onclick={() =>
              authState.userCode &&
              navigator.clipboard.writeText(authState.userCode)}
            >{authState.userCode ?? 'WAIT'}</button
          >
          <p class="mt-3 text-xs text-(--muted)">
            {authState.message ??
              'Approve read-only followed-channel access in the browser.'}
          </p>
          {#if authState.verificationUri}
            <p class="mt-2 font-mono text-[0.65rem] break-all text-(--muted)">
              {authState.verificationUri}
            </p>
            <div class="mt-4 flex justify-center gap-2">
              <Button
                variant="secondary"
                onclick={() =>
                  navigator.clipboard.writeText(authState.userCode ?? '')}
                ><Copy class="size-3.5" />Copy code</Button
              >
              <Button
                variant="secondary"
                onclick={() => open(authState.verificationUri ?? '')}
                ><ExternalLink class="size-3.5" />Open verification page</Button
              >
              <Button variant="ghost" onclick={cancelAuth}
                ><X class="size-3.5" />Cancel</Button
              >
            </div>
          {/if}
        </section>
      {:else if !['idle', 'success'].includes(authState.status)}
        <EmptyState
          eyebrow="AUTH / TWITCH"
          title={authFailureTitle}
          message={authState.message ?? 'The account was not connected.'}
          actionLabel="Try again"
          onAction={connect}
        />
      {:else if !clientConfigured}
        <EmptyState
          eyebrow="SETUP / TWITCH"
          title="Twitch client ID is not configured"
          message="Add the Public Twitch application client ID in Settings before Twitch can connect."
          actionLabel="Open Settings"
          onAction={() => onNavigateSettings('twitch')}
        />
      {:else}
        <EmptyState
          eyebrow="ACCOUNT / TWITCH"
          title="Connect a Twitch account"
          message="Thelxinoe requests followed-channel access only and stores encrypted tokens on your server."
          actionLabel="Connect Twitch"
          onAction={connect}
        />
      {/if}
    {:else if loading && streams.length === 0}
      <EmptyState
        eyebrow="CACHE / READ"
        title="Loading the live snapshot"
        message="Thelxinoe is reading the latest saved snapshot while refresh runs."
      />
    {:else if streams.length === 0}
      {#if syncStatus.error}
        <EmptyState
          eyebrow="SYNC / TWITCH"
          title={syncStatus.rateLimitedUntil
            ? 'Twitch rate limit reached'
            : syncStatus.error.code.includes('auth')
              ? 'Twitch authorization expired'
              : 'Twitch API unavailable'}
          message={syncStatus.error.message}
          actionLabel="Retry"
          onAction={refresh}
        />
      {:else}
        <EmptyState
          eyebrow="FOLLOWED / OFFLINE"
          title="No followed channels are live"
          message="The snapshot contains only followed channels that are streaming now. It refreshes every minute."
          actionLabel="Refresh now"
          onAction={refresh}
        />
      {/if}
    {:else}
      <div class="pb-4" data-sidebar-resize="y">
        <section data-card-grid class="media-card-grid grid items-stretch">
          {#each streams as stream (stream.streamId)}
            <div
              data-layout-key={`twitch-stream:${stream.streamId}`}
              class="h-full min-w-0"
            >
              <StreamCard
                {stream}
                {playbackReady}
                isPlaying={activeChannelLogins.has(stream.login.toLowerCase())}
                isLaunching={launchingChannelLogins.has(
                  stream.login.toLowerCase(),
                )}
                onWatch={watch}
                onCopy={copy}
                onOpen={(item) => open(twitchUrl(item.login))}
                onChat={(item) => open(twitchChatUrl(item.login))}
              />
            </div>
          {/each}
        </section>
      </div>
    {/if}
  </div>
</div>
