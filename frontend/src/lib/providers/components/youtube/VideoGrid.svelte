<script lang="ts">
  import { untrack } from 'svelte';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import type {
    YoutubeCardShortcut,
    YoutubeDownloadEvent,
    YoutubeWatchlist,
  } from '../../types';
  import type { YoutubeVideoGroup } from '../../youtube-feed';
  import VideoCard from './VideoCard.svelte';

  let {
    actions,
    groups,
    showGroupHeaders,
    cardColumns,
    fadeWatchedCards,
    youtubeCardShortcuts,
    signedInPlaybackAvailable,
    mpvReady,
    ytdlpReady,
    ffmpegReady,
    activeVideoIds,
    launchingVideoIds,
    downloadBusyIds,
    downloadProgress,
    watchlists,
    hasMore,
    loadingMore,
    onLoadMore,
  }: {
    actions: YoutubeVideoActions;
    groups: YoutubeVideoGroup[];
    showGroupHeaders: boolean;
    cardColumns: number;
    fadeWatchedCards: boolean;
    youtubeCardShortcuts: YoutubeCardShortcut[];
    signedInPlaybackAvailable: boolean;
    mpvReady: boolean;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    activeVideoIds: Set<string>;
    launchingVideoIds: Set<string>;
    downloadBusyIds: Set<string>;
    downloadProgress: Map<string, YoutubeDownloadEvent>;
    watchlists: YoutubeWatchlist[];
    hasMore: boolean;
    loadingMore: boolean;
    onLoadMore: () => void;
  } = $props();

  let loadMoreTarget = $state<HTMLDivElement | null>(null);
  let openMenuVideoId = $state<string | null>(null);
  let confirmingShortcutDeleteVideoId = $state<string | null>(null);
  let observedCardColumns = $state(untrack(() => cardColumns));

  function closeCardActions() {
    openMenuVideoId = null;
    confirmingShortcutDeleteVideoId = null;
  }

  function toggleMenu(videoId: string) {
    confirmingShortcutDeleteVideoId = null;
    openMenuVideoId = openMenuVideoId === videoId ? null : videoId;
  }

  $effect(() => {
    const next = cardColumns;
    if (next === observedCardColumns) return;
    observedCardColumns = next;
    closeCardActions();
  });

  function trackStickyGroupHeader(node: HTMLElement) {
    const root = node.closest<HTMLElement>('[data-feed-scroll]');
    const section = node.closest<HTMLElement>('section');
    if (!root || !section) return;
    const scrollRoot: HTMLElement = root;
    const groupSection: HTMLElement = section;

    let frame: number | null = null;

    function update() {
      frame = null;
      const rootTop =
        scrollRoot.getBoundingClientRect().top +
        (Number.parseFloat(getComputedStyle(scrollRoot).paddingTop) || 0);
      const headerTop = node.getBoundingClientRect().top;
      const sectionTop = groupSection.getBoundingClientRect().top;
      node.toggleAttribute(
        'data-stuck',
        sectionTop < rootTop && Math.abs(headerTop - rootTop) <= 1,
      );
    }

    function scheduleUpdate() {
      if (frame === null) frame = requestAnimationFrame(update);
    }

    scrollRoot.addEventListener('scroll', scheduleUpdate, { passive: true });
    window.addEventListener('resize', scheduleUpdate);
    update();
    return {
      destroy() {
        scrollRoot.removeEventListener('scroll', scheduleUpdate);
        window.removeEventListener('resize', scheduleUpdate);
        if (frame !== null) cancelAnimationFrame(frame);
        node.removeAttribute('data-stuck');
      },
    };
  }

  $effect(() => {
    const target = loadMoreTarget;
    if (!target || !hasMore || loadingMore) return;
    const root = target.closest<HTMLElement>('[data-feed-scroll]');
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) onLoadMore();
      },
      { root, rootMargin: '400px 0px' },
    );
    observer.observe(target);
    return () => observer.disconnect();
  });
</script>

{#if openMenuVideoId || confirmingShortcutDeleteVideoId}
  <button
    type="button"
    class="fixed inset-0 z-40 cursor-default bg-transparent"
    aria-label={confirmingShortcutDeleteVideoId
      ? 'Cancel deleting download'
      : 'Close video actions'}
    onclick={closeCardActions}
  ></button>
{/if}

{#each groups as group (group.key)}
  <section class="pb-2">
    {#if showGroupHeaders}
      <h2
        use:trackStickyGroupHeader
        data-card-group-header
        data-sidebar-resize="y-pos"
        data-layout-key={`youtube-group:${group.key}`}
        class="sticky top-0 z-10 border-b border-(--line) bg-transparent py-2 text-xs font-semibold tracking-[0.08em] text-(--foreground) uppercase data-stuck:bg-(--background)/95 data-stuck:backdrop-blur"
      >
        <span class="feed-group-label">{group.label}</span>
      </h2>
    {/if}
    <div class="pt-3" data-sidebar-resize="y">
      <div data-card-grid class="media-card-grid grid items-stretch">
        {#each group.videos as video (video.videoId)}
          <div
            data-layout-key={`youtube-video:${video.videoId}`}
            class="h-full min-w-0"
          >
            <VideoCard
              {actions}
              {video}
              {mpvReady}
              {ytdlpReady}
              {ffmpegReady}
              {fadeWatchedCards}
              {youtubeCardShortcuts}
              {signedInPlaybackAvailable}
              isPlaying={activeVideoIds.has(video.videoId)}
              isLaunching={launchingVideoIds.has(video.videoId)}
              downloadBusy={downloadBusyIds.has(video.videoId)}
              downloadEvent={downloadProgress.get(video.videoId)}
              {watchlists}
              menuOpen={openMenuVideoId === video.videoId}
              shortcutDeleteConfirming={confirmingShortcutDeleteVideoId ===
                video.videoId}
              onMenuToggle={() => toggleMenu(video.videoId)}
              onShortcutDeleteRequest={() => {
                openMenuVideoId = null;
                confirmingShortcutDeleteVideoId = video.videoId;
              }}
              onShortcutDeleteCancel={() =>
                (confirmingShortcutDeleteVideoId = null)}
            />
          </div>
        {/each}
      </div>
    </div>
  </section>
{/each}

<div bind:this={loadMoreTarget} class="h-px" aria-hidden="true"></div>
{#if loadingMore}
  <p class="eyebrow sticky bottom-2 mx-auto w-fit bg-(--surface) px-3 py-2">
    Loading cached feed…
  </p>
{/if}
