<script lang="ts">
  import { LatestRequest } from '../../latest-request';
  import { untrack } from 'svelte';
  import { Plus, RefreshCw, Settings2, Trash2 } from '@lucide/svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { api, normalizeError } from '../../api';
  import { createCardGridWheelHandler } from '../../../card-grid-wheel';
  import { scaleCardScope } from '../../../card-grid-zoom';
  import { createLayoutMotion } from '../../../layout-motion';
  import { dismissToastByKey, showToast } from '../../toasts';
  import type {
    AppError,
    KickChannel,
    PlaybackDiagnostics,
    PlaybackSession,
    SyncStatus,
  } from '../../types';
  import { isPlaybackLaunching, kickUrl, relativeTime } from '../../utils';
  import Button from '../../../ui/Button.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import EmptyState from '../ui/EmptyState.svelte';
  import ProviderConnection from '../../ProviderConnection.svelte';
  import { api as request } from '../../../api';
  import { eyebrowTextClass as eyebrowClass } from '../../../ui/styles';
  import KickCard from './KickCard.svelte';

  let {
    admin,
    connected,
    onAccountChanged,
    syncStatus,
    playback,
    activeSessions,
    cardColumns,
    metadataConfigured,
    dataRevision,
    onNavigateSettings,
    onCardColumnsChange,
  }: {
    admin: boolean;
    connected: boolean;
    onAccountChanged: () => void;
    syncStatus: SyncStatus;
    playback: PlaybackDiagnostics;
    activeSessions: PlaybackSession[];
    cardColumns: number;
    metadataConfigured: boolean;
    dataRevision: number;
    onNavigateSettings: (section: 'kick' | 'tools') => void;
    onCardColumnsChange: (columns: number) => void;
  } = $props();

  let channels = $state<KickChannel[]>([]);
  let loading = $state(true);
  let error = $state<AppError | null>(null);
  let managerOpen = $state(false);
  let channelInput = $state('');
  let adding = $state(false);
  let removalTarget = $state<KickChannel | null>(null);
  let removing = $state(false);
  let connecting = $state(false);
  async function resumeTracking() {
    connecting = true;
    try {
      await request('/online/kick/connect', 'POST');
      onAccountChanged();
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      connecting = false;
    }
  }
  let observedDataRevision = $state<number | null>(null);
  const layoutMotion = createLayoutMotion();
  const connectLayoutMotion = layoutMotion.connect;
  const requests = new LatestRequest();
  const pendingSlugs = new SvelteSet<string>();
  const playbackReady = $derived(
    playback.mpv.detected && playback.streamlink.detected,
  );
  const activeSlugs = $derived(
    new Set(
      activeSessions
        .filter((session) => session.platform === 'kick')
        .map((session) => session.mediaId.toLowerCase()),
    ),
  );
  const launchingSlugs = $derived(
    new Set([
      ...pendingSlugs,
      ...activeSessions
        .filter(
          (session) =>
            session.platform === 'kick' &&
            isPlaybackLaunching(session.playbackState),
        )
        .map((session) => session.mediaId.toLowerCase()),
    ]),
  );
  const displayedChannels = $derived(
    [...channels].sort((left, right) => {
      const leftViewers = left.liveStream?.viewerCount;
      const rightViewers = right.liveStream?.viewerCount;
      if (leftViewers !== undefined && rightViewers !== undefined)
        return rightViewers - leftViewers;
      if (leftViewers !== undefined) return -1;
      if (rightViewers !== undefined) return 1;
      return left.slug.localeCompare(right.slug);
    }),
  );
  const hasLiveMetadata = $derived(
    channels.some((channel) => channel.liveStream),
  );
  const liveMetadataIsFresh = $derived(
    metadataConfigured &&
      Boolean(syncStatus.lastSuccessAt) &&
      !syncStatus.isRefreshing &&
      !syncStatus.stale,
  );

  $effect(() => {
    const expectedConfiguration = metadataConfigured;
    untrack(() => void load(expectedConfiguration));
  });

  $effect(() => {
    const revision = dataRevision;
    if (observedDataRevision === null) {
      observedDataRevision = revision;
      return;
    }
    if (revision === observedDataRevision) return;
    observedDataRevision = revision;
    untrack(() => void load());
  });

  $effect(() => {
    if (channels.length > 0 && !playbackReady) {
      showToast({
        key: 'kick-playback-missing',
        tone: 'warning',
        title: 'Playback dependencies missing',
        message: 'Configure the Windows player in Settings to watch channels.',
        actionLabel: 'Player settings',
        onAction: () => onNavigateSettings('tools'),
        durationMs: null,
      });
    } else {
      dismissToastByKey('kick-playback-missing');
    }
  });

  $effect(() => {
    const syncError = syncStatus.error;
    if (!syncError) {
      dismissToastByKey('kick-sync-error');
      return;
    }
    showToast({
      key: 'kick-sync-error',
      tone: syncStatus.rateLimitedUntil ? 'warning' : 'error',
      title: syncStatus.rateLimitedUntil
        ? 'Kick rate limit reached'
        : 'Kick refresh failed',
      message: `${syncError.message}${hasLiveMetadata ? ` Cached streams remain available. Last good sync ${relativeTime(syncStatus.lastSuccessAt)}.` : ''}`,
      detail: syncError.technical,
      actionLabel:
        syncStatus.rateLimitedUntil || !metadataConfigured ? null : 'Retry',
      onAction:
        syncStatus.rateLimitedUntil || !metadataConfigured ? null : refresh,
      durationMs: syncStatus.rateLimitedUntil ? null : 10_000,
    });
  });

  $effect(() => {
    if (!error) {
      dismissToastByKey('kick-action-error');
      return;
    }
    showToast({
      key: 'kick-action-error',
      tone: 'error',
      title: 'Kick action failed',
      message: error.message,
      detail: error.technical,
      durationMs: null,
    });
  });

  async function load(expectedConfiguration = metadataConfigured) {
    const current = requests.begin();
    loading = channels.length === 0;
    error = null;
    try {
      const snapshot = await api.kickSnapshot();
      if (expectedConfiguration !== metadataConfigured || !current()) return;
      channels = snapshot.channels;
    } catch (caught) {
      if (current()) error = normalizeError(caught);
    } finally {
      if (current()) loading = false;
    }
  }

  async function refresh() {
    error = null;
    try {
      await api.syncKick();
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  async function addChannel(event: SubmitEvent) {
    event.preventDefault();
    const value = channelInput.trim();
    if (!value || adding) return;
    adding = true;
    error = null;
    try {
      const added = await api.addKickTrackedChannel(value);
      channelInput = '';
      await load();
      onAccountChanged();
      showToast({
        key: 'kick-channel-added',
        tone: 'success',
        title: `${added.displayName} is now tracked`,
        message: metadataConfigured
          ? 'Live metadata refresh started.'
          : 'The channel is ready for direct playback.',
      });
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      adding = false;
    }
  }

  async function removeChannel() {
    const channel = removalTarget;
    removalTarget = null;
    if (!channel || removing) return;
    removing = true;
    error = null;
    try {
      await api.removeKickTrackedChannel(channel.slug);
      await load();
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      removing = false;
    }
  }

  async function watch(channel: KickChannel) {
    const slug = channel.slug.toLowerCase();
    if (pendingSlugs.has(slug) || activeSlugs.has(slug)) return;
    error = null;
    pendingSlugs.add(slug);
    try {
      await api.launchKick(channel.slug);
    } catch (caught) {
      error = normalizeError(caught);
    } finally {
      pendingSlugs.delete(slug);
    }
  }

  async function copy(channel: KickChannel) {
    error = null;
    try {
      await navigator.clipboard.writeText(kickUrl(channel.slug));
    } catch (caught) {
      error = normalizeError(caught);
    }
  }

  const handleCardGridWheel = createCardGridWheelHandler({
    anchorAttribute: 'data-channel-slug',
    motion: layoutMotion,
    getColumns: () => cardColumns,
    setColumns: (columns) => onCardColumnsChange(columns),
  });

  async function open(channel: KickChannel) {
    error = null;
    try {
      await api.openExternal(kickUrl(channel.slug));
    } catch (caught) {
      error = normalizeError(caught);
    }
  }
</script>

<div class="flex flex-col">
  <div
    class="relative mr-3 flex shrink-0 flex-wrap items-center gap-2 border-b border-transparent pb-2"
  >
    <span
      data-sidebar-resize="xy"
      aria-hidden="true"
      class="pointer-events-none absolute right-0 -bottom-px left-0 h-px bg-line"
    ></span>
    <Button
      data-sidebar-resize="xy"
      size="sm"
      variant="ghost"
      aria-expanded={managerOpen}
      onclick={() => (managerOpen = !managerOpen)}
    >
      <Settings2 class="size-3.5" />Manage channels
    </Button>
    <span data-sidebar-resize="xy" class="text-[0.65rem] text-muted">
      {channels.length} tracked
    </span>
    <span data-sidebar-resize="xy" class="ml-auto text-[0.65rem] text-muted">
      {#if syncStatus.isRefreshing}{syncStatus.phase} · {syncStatus.completed}/{syncStatus.total ??
          '—'}{:else if metadataConfigured}Metadata updated {relativeTime(
          syncStatus.lastSuccessAt,
        )}{:else}Live metadata is optional{/if}
    </span>
    <Button
      data-sidebar-resize="xy"
      size="sm"
      variant="secondary"
      disabled={!metadataConfigured ||
        syncStatus.isRefreshing ||
        channels.length === 0}
      onclick={refresh}
      title={metadataConfigured
        ? 'Refresh Kick live metadata'
        : 'Add an optional Kick API client in Settings to refresh live metadata'}
    >
      <RefreshCw
        class={`size-3.5 ${syncStatus.isRefreshing ? 'animate-spin' : ''}`}
      />Refresh metadata
    </Button>
  </div>

  {#if managerOpen}
    <section
      data-sidebar-resize="x"
      class="mt-3 mr-3 shrink-0 border border-line bg-surface-soft p-3 shadow-panel"
      aria-label="Tracked Kick channels"
    >
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p class={eyebrowClass}>TRACKED CHANNELS / KICK</p>
          <p class="mt-1 max-w-2xl text-xs leading-5 text-muted">
            This private list does not need a Kick sign-in or API client. Add
            each channel once by slug or <span class="font-mono"
              >https://kick.com/&lt;channel&gt;</span
            >.
          </p>
        </div>
        <form
          class="flex min-w-72 flex-1 gap-2 sm:max-w-xl"
          onsubmit={addChannel}
        >
          <label class="sr-only" for="kick-channel-input">Kick channel</label>
          <input
            id="kick-channel-input"
            class="min-h-9 min-w-0 flex-1 border border-line bg-surface px-3 text-xs text-foreground focus:border-line-strong"
            type="text"
            placeholder="Channel slug or Kick URL"
            autocomplete="off"
            spellcheck="false"
            disabled={adding}
            bind:value={channelInput}
          />
          <Button
            size="sm"
            type="submit"
            disabled={adding || !channelInput.trim()}
          >
            <Plus class="size-3.5" />{adding ? 'Adding…' : 'Add'}
          </Button>
        </form>
      </div>
      {#if channels.length > 0}
        <ul class="mt-3 grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {#each channels as channel (channel.slug)}
            <li
              class="flex min-w-0 items-center gap-2 border border-line bg-surface p-2"
            >
              {#if channel.profilePictureUrl}
                <img
                  class="size-8 shrink-0 rounded-full object-cover"
                  src={channel.profilePictureUrl}
                  alt=""
                  loading="lazy"
                />
              {:else}
                <span
                  class="grid size-8 shrink-0 place-items-center rounded-full border border-line text-xs text-[#72ff43]"
                  >{channel.displayName.slice(0, 1).toUpperCase()}</span
                >
              {/if}
              <span class="min-w-0 flex-1">
                <strong class="block truncate text-xs"
                  >{channel.displayName}</strong
                >
                <span class="block truncate font-mono text-[0.6rem] text-muted"
                  >kick.com/{channel.slug}</span
                >
              </span>
              <Button
                size="icon"
                class="size-8"
                variant="ghost"
                aria-label={`Stop tracking ${channel.displayName}`}
                disabled={removing ||
                  activeSlugs.has(channel.slug.toLowerCase())}
                onclick={() => (removalTarget = channel)}
              >
                <Trash2 class="size-3.5" />
              </Button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}

  <div
    use:connectLayoutMotion
    use:scaleCardScope={cardColumns}
    onwheel={handleCardGridWheel}
    data-sidebar-resize="x"
    class="relative pt-3 pr-3"
    data-feed-content
  >
    {#if loading}
      <EmptyState
        eyebrow="CACHE / READ"
        title="Loading Kick channels"
        message="Thelxinoe is reading the tracked list and latest live snapshot."
      />
    {:else if error && channels.length === 0}
      <EmptyState
        eyebrow="KICK / ERROR"
        title="Kick action failed"
        message={error.message}
        actionLabel="Retry"
        onAction={load}
      />
    {:else if channels.length === 0}
      <ProviderConnection
        platform="kick"
        {admin}
        configured={metadataConfigured}
        onConfigured={onAccountChanged}
        onConnect={() => (managerOpen = true)}
      />
    {:else}
      {#if !connected}
        <div
          class="mb-3 flex flex-wrap items-center justify-between gap-2 border border-line bg-surface-soft px-3 py-2 text-xs text-muted"
        >
          <span>Tracking is paused. Resume to refresh your saved channels.</span
          >
          <Button size="sm" disabled={connecting} onclick={resumeTracking}
            >Resume tracking</Button
          >
          {#if error}<p role="alert">{error.message}</p>{/if}
        </div>
      {/if}
      {#if !metadataConfigured}
        <div class="mb-3">
          <ProviderConnection
            platform="kick"
            {admin}
            configured={metadataConfigured}
            onConfigured={onAccountChanged}
          />
        </div>
      {/if}
      <div class="pb-4" data-sidebar-resize="y">
        <section
          data-card-grid
          class="grid items-stretch [contain:layout_style]"
          aria-label="Tracked Kick channels"
        >
          {#each displayedChannels as channel (channel.slug)}
            <div
              data-layout-key={`kick-channel:${channel.slug}`}
              class="h-full min-w-0"
            >
              <KickCard
                {channel}
                {playbackReady}
                offlineConfirmed={liveMetadataIsFresh && !channel.liveStream}
                isPlaying={activeSlugs.has(channel.slug.toLowerCase())}
                isLaunching={launchingSlugs.has(channel.slug.toLowerCase())}
                onWatch={watch}
                onCopy={copy}
                onOpen={open}
              />
            </div>
          {/each}
        </section>
      </div>
    {/if}
  </div>
</div>

<ConfirmDialog
  open={removalTarget !== null}
  title={`Stop tracking ${removalTarget?.displayName ?? 'this Kick channel'}?`}
  message="This removes the channel from your tracked list and clears its live metadata. It does not change your Kick account."
  confirmLabel="Stop tracking"
  danger
  onConfirm={removeChannel}
  onCancel={() => (removalTarget = null)}
/>
