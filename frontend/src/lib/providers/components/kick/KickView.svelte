<script lang="ts">
  import MediaSkeleton from '../../../ui/MediaSkeleton.svelte';
  import { LatestRequest } from '../../../latest-request';
  import { untrack } from 'svelte';
  import { Plus, RefreshCw } from '@lucide/svelte';
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
  import { api as request } from '../../../api';
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
      data-sidebar-resize="x"
      aria-hidden="true"
      class="pointer-events-none absolute right-0 -bottom-px left-0 h-px bg-line"
    ></span>
    <form
      data-sidebar-resize="xy-pos"
      class="flex w-full min-w-0 items-center gap-2 sm:w-80"
      onsubmit={addChannel}
    >
      <label class="sr-only" for="kick-channel-input">Kick channel</label>
      <input
        id="kick-channel-input"
        class="h-8 min-w-0 flex-1 border border-line bg-surface px-3 text-xs text-foreground focus:border-line-strong"
        type="text"
        placeholder="Channel slug or Kick URL"
        autocomplete="off"
        spellcheck="false"
        disabled={adding}
        bind:value={channelInput}
      />
      <Button
        size="sm"
        class="shrink-0"
        type="submit"
        disabled={adding || !channelInput.trim()}
      >
        <Plus class="size-3.5" />{adding ? 'Adding…' : 'Add'}
      </Button>
    </form>
    {#if !metadataConfigured}
      <p data-sidebar-resize="xy-pos" class="text-[0.65rem] text-muted">
        {#if admin}
          <a
            href="#admin/providers"
            class="text-accent underline underline-offset-2 hover:text-foreground"
            onclick={(event) => {
              event.preventDefault();
              onNavigateSettings('kick');
            }}>Show live status, titles and thumbnails</a
          >
        {:else}
          Ask your administrator to set up Kick to show live status, titles and
          thumbnails.
        {/if}
      </p>
    {:else}
      <span
        data-sidebar-resize="xy-pos"
        class="ml-auto text-[0.65rem] text-muted"
      >
        {#if syncStatus.isRefreshing}{syncStatus.phase} · {syncStatus.completed}/{syncStatus.total ??
            '—'}{:else}Metadata updated {relativeTime(
            syncStatus.lastSuccessAt,
          )}{/if}
      </span>
      <Button
        data-sidebar-resize="xy-pos"
        size="sm"
        variant="secondary"
        class="ml-auto shrink-0 sm:ml-0"
        disabled={syncStatus.isRefreshing || channels.length === 0}
        onclick={refresh}
        title="Refresh Kick live metadata"
      >
        <RefreshCw
          class={`size-3.5 ${syncStatus.isRefreshing ? 'animate-spin' : ''}`}
        />Refresh metadata
      </Button>
    {/if}
  </div>

  <div
    use:connectLayoutMotion
    use:scaleCardScope={cardColumns}
    onwheel={handleCardGridWheel}
    data-sidebar-resize="xy"
    class="relative pt-3 pr-3"
    data-feed-content
  >
    {#if loading}
      <MediaSkeleton
        label="Loading Kick channels"
        layout="feed"
        shape="landscape"
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
      <p class="py-12 text-center text-sm text-muted">No tracked channels.</p>
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
      <div class="pb-4" data-sidebar-resize="y-pos">
        <section
          data-card-grid
          data-sidebar-resize="media"
          data-sidebar-group
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
                removeDisabled={removing ||
                  activeSlugs.has(channel.slug.toLowerCase())}
                onWatch={watch}
                onCopy={copy}
                onOpen={open}
                onRemove={() => (removalTarget = channel)}
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
