<script lang="ts">
  import { untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import type {
    YoutubeCardShortcut,
    YoutubeDownloadEvent,
    YoutubeWatchlist,
  } from '../../types';
  import type { YoutubeVideoGroup } from '../../youtube-feed';
  import { eyebrowTextClass as eyebrowClass } from '../../../ui/styles';
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

  const stickyHeaders = new SvelteMap<HTMLElement, HTMLElement>();
  let stickyRoot: HTMLElement | null = null;
  let stickyShell: HTMLElement | null = null;
  let stickyResize: ResizeObserver | null = null;
  let stickyFrame: number | null = null;

  function updateStickyHeaders() {
    stickyFrame = null;
    if (!stickyRoot) return;
    const rootTop =
      stickyRoot.getBoundingClientRect().top +
      (Number.parseFloat(getComputedStyle(stickyRoot).paddingTop) || 0);
    const states = [...stickyHeaders].map(([header, section]) => ({
      header,
      stuck:
        section.getBoundingClientRect().top < rootTop &&
        Math.abs(header.getBoundingClientRect().top - rootTop) <= 1,
    }));
    for (const { header, stuck } of states)
      header.toggleAttribute('data-stuck', stuck);
  }

  function scheduleStickyHeaders() {
    if (stickyFrame === null)
      stickyFrame = requestAnimationFrame(updateStickyHeaders);
  }

  function trackStickyGroupHeader(node: HTMLElement) {
    const root = node.closest<HTMLElement>('[data-feed-scroll]');
    const section = node.closest<HTMLElement>('section');
    if (!root || !section) return;
    if (!stickyRoot) {
      stickyRoot = root;
      stickyShell = root.closest('[data-sidebar-shell]');
      stickyResize = new ResizeObserver(scheduleStickyHeaders);
      stickyResize.observe(root);
      root.addEventListener('scroll', scheduleStickyHeaders, { passive: true });
      stickyShell?.addEventListener(
        'sidebar-motion-end',
        scheduleStickyHeaders,
      );
    }
    stickyHeaders.set(node, section);
    stickyResize?.observe(section);
    scheduleStickyHeaders();
    return {
      destroy() {
        stickyHeaders.delete(node);
        stickyResize?.unobserve(section);
        node.removeAttribute('data-stuck');
        if (stickyHeaders.size > 0) return;
        stickyRoot?.removeEventListener('scroll', scheduleStickyHeaders);
        stickyShell?.removeEventListener(
          'sidebar-motion-end',
          scheduleStickyHeaders,
        );
        stickyResize?.disconnect();
        if (stickyFrame !== null) cancelAnimationFrame(stickyFrame);
        stickyFrame = null;
        stickyRoot = null;
        stickyShell = null;
        stickyResize = null;
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
        data-sidebar-resize="xy"
        data-layout-key={`youtube-group:${group.key}`}
        class="sticky top-0 z-10 border-b border-line bg-transparent py-2 text-xs font-semibold tracking-[0.08em] text-foreground uppercase data-stuck:bg-background/95 data-stuck:backdrop-blur"
      >
        <span data-sidebar-resize="xy-pos" class="feed-group-label"
          >{group.label}</span
        >
      </h2>
    {/if}
    <div class="pt-3" data-sidebar-resize="y-pos">
      <div
        data-card-grid
        data-sidebar-resize="media"
        data-sidebar-group
        class="grid items-stretch [contain:layout_style]"
      >
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
  <p
    class={`${eyebrowClass} sticky bottom-2 mx-auto w-fit bg-surface px-3 py-2`}
  >
    Loading cached feed…
  </p>
{/if}
