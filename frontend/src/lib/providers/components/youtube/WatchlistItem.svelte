<script lang="ts">
  import ProgressBar from '../../../ui/ProgressBar.svelte';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { youtubeVideoPresentation } from '../../youtube-video-presentation';
  import { Download, Eye, EyeOff, Trash2, X } from '@lucide/svelte';
  import type { YoutubeDownloadEvent, YoutubeVideo } from '../../types';
  import { formatClock, youtubeThumbnailUrl } from '../../utils';

  let {
    actions,
    video,
    downloadEvent,
    downloadBusy,
    isPlaying = false,
    isLaunching = false,
    mpvReady,
    ytdlpReady,
    ffmpegReady,
    sortable = false,
    dragging = false,
    suppressClick = false,
    onRemove,
  }: {
    actions: YoutubeVideoActions;
    video: YoutubeVideo;
    downloadEvent?: YoutubeDownloadEvent;
    downloadBusy: boolean;
    isPlaying?: boolean;
    isLaunching?: boolean;
    mpvReady: boolean;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    sortable?: boolean;
    dragging?: boolean;
    suppressClick?: boolean;
    onRemove: (video: YoutubeVideo) => void;
  } = $props();

  const presentation = $derived(
    youtubeVideoPresentation(video, {
      mpvReady,
      ytdlpReady,
      ffmpegReady,
      isPlaying,
      isLaunching,
      downloadBusy,
      downloadEvent,
    }),
  );
  const activeDownload = $derived(presentation.activeDownload);
  const downloadReady = $derived(presentation.downloadReady);
  const downloadPending = $derived(presentation.downloadPending);
  const canPlay = $derived(presentation.canPlay);
  const canDownload = $derived(presentation.canDownload);
  const isBusy = $derived(presentation.isBusy);
  const tracksVideoProgress = $derived(presentation.tracksVideoProgress);

  const showPosition = $derived(presentation.showPosition);
  const remainingSeconds = $derived(presentation.remainingSeconds);
  const downloadPercent = $derived(presentation.downloadPercent);
  const progressPercent = $derived(presentation.progressPercent);
  const downloadLabel = $derived(
    presentation.downloadLabel ??
      (downloadReady
        ? (presentation.downloadFormatLabel ?? activeDownload?.quality)
        : null),
  );
  let thumbnailAspectRatio = $state(16 / 9);
  let titleTruncated = $state(false);

  function trackTitleOverflow(node: HTMLElement) {
    const update = () => {
      titleTruncated = node.scrollHeight > node.clientHeight + 1;
    };
    const resizeObserver = new ResizeObserver(update);
    const mutationObserver = new MutationObserver(update);
    resizeObserver.observe(node);
    mutationObserver.observe(node, {
      childList: true,
      characterData: true,
      subtree: true,
    });
    requestAnimationFrame(update);

    return {
      destroy() {
        resizeObserver.disconnect();
        mutationObserver.disconnect();
      },
    };
  }

  function updateThumbnailRatio(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    if (image.naturalWidth > 0 && image.naturalHeight > 0) {
      thumbnailAspectRatio = image.naturalWidth / image.naturalHeight;
    }
  }

  function clickCard(event: MouseEvent) {
    if (
      suppressClick ||
      !canPlay ||
      (event.target as HTMLElement).closest('button')
    )
      return;
    actions.onPlay(video);
  }

  function keyCard(event: KeyboardEvent) {
    if (
      event.target !== event.currentTarget ||
      !canPlay ||
      !['Enter', ' '].includes(event.key)
    )
      return;
    event.preventDefault();
    actions.onPlay(video);
  }
</script>

<div
  role="button"
  tabindex="0"
  aria-disabled={!canPlay}
  data-playing={isPlaying}
  aria-label={isPlaying ? `Playing: ${video.title}` : video.title}
  class={`group/watchlist-item relative flex cursor-default items-stretch border-b border-line transition-[opacity,background-color] duration-150 hover:z-20 ${sortable ? (dragging ? 'bg-accent-soft opacity-35 select-none' : 'select-none hover:bg-surface-soft') : canPlay ? 'hover:bg-surface-soft' : ''}`}
  onclick={clickCard}
  onkeydown={keyCard}
>
  {#if isPlaying}
    <span
      aria-hidden="true"
      class="pointer-events-none absolute inset-y-0 right-0 z-10 w-1 bg-accent"
    ></span>
  {/if}
  <div
    class="relative w-[min(10rem,42%)] shrink-0 overflow-hidden bg-[linear-gradient(130deg,#121923,#080b10_60%)]"
    style={`aspect-ratio:${thumbnailAspectRatio}`}
  >
    {#if video.thumbnailUrl}
      <img
        class="size-full object-contain"
        src={youtubeThumbnailUrl(video.thumbnailUrl)}
        alt=""
        draggable="false"
        onload={updateThumbnailRatio}
      />
    {/if}
    {#if showPosition}
      <span
        class="absolute bottom-2 left-1 border border-accent/45 bg-black/75 px-1.5 py-0.5 font-mono text-[0.6rem] text-accent"
      >
        {formatClock(video.positionSeconds)}
      </span>
    {/if}
    {#if tracksVideoProgress && video.durationSeconds}
      <span
        class="absolute right-1 bottom-2 border border-white/20 bg-black/75 px-1.5 py-0.5 font-mono text-[0.6rem] text-white"
      >
        {isPlaying && remainingSeconds !== null
          ? `-${formatClock(remainingSeconds)}`
          : formatClock(video.durationSeconds)}
      </span>
    {/if}
    {#if downloadPending && downloadPercent !== null}
      <ProgressBar
        value={downloadPercent}
        class="absolute inset-x-0 bottom-1 bg-success/18 text-success"
        barClass="shadow-[0_0_12px_var(--success)] transition-[width] duration-150 motion-reduce:transition-none"
      />
    {/if}
    {#if tracksVideoProgress && progressPercent > 0}
      <ProgressBar
        value={progressPercent}
        class="absolute inset-x-0 bottom-0 bg-white/12"
        barClass="shadow-accent-glow"
      />
    {/if}
  </div>

  <div class="relative flex min-w-0 flex-1 flex-col px-2.5 py-1">
    <div class="group/title relative min-w-0">
      <p
        use:trackTitleOverflow
        class="line-clamp-2 text-sm leading-5 font-medium"
      >
        {video.title}
      </p>
      {#if titleTruncated && !dragging}
        <div
          class="pointer-events-none absolute -top-1.5 -right-1.5 -left-1.5 z-30 translate-y-0.5 border border-line-strong bg-surface-strong px-2 py-1.5 text-sm leading-5 font-medium opacity-0 shadow-[0_12px_30px_var(--shadow)] transition-[opacity,translate] delay-0 duration-150 ease-out group-hover/title:translate-y-0 group-hover/title:opacity-100 group-hover/title:delay-750 motion-reduce:transition-none"
          aria-hidden="true"
        >
          {video.title}
        </div>
      {/if}
    </div>
    <p
      class="mt-0.5 truncate text-[0.62rem] tracking-[0.08em] text-muted uppercase"
    >
      {video.channelName}
    </p>
    <span
      class={`mt-auto min-w-0 truncate pr-24 text-[0.58rem] font-bold tracking-[0.1em] uppercase ${downloadLabel && !['failed', 'interrupted'].includes(activeDownload?.status ?? '') ? 'text-success' : activeDownload?.status === 'failed' ? 'text-danger' : 'text-muted'}`}
    >
      {#if downloadReady && downloadLabel}
        <span class="inline-flex items-center gap-1">
          <Download class="size-3 shrink-0" />
          {downloadLabel}
        </span>
      {:else if activeDownload?.status === 'downloading'}
        <span class="inline-flex items-center gap-1">
          <Download class="size-3 shrink-0" />
          {downloadPercent === null ? '…' : `${Math.round(downloadPercent)}%`}
        </span>
      {:else}
        {downloadLabel ?? 'Remote'}
      {/if}
    </span>
  </div>

  <div
    class="invisible absolute right-1 bottom-1 flex gap-0.5 group-hover/watchlist-item:visible group-focus-visible/watchlist-item:visible group-has-focus-visible/watchlist-item:visible"
  >
    <button
      type="button"
      class="grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-foreground"
      title={video.isWatched ? 'Mark unwatched' : 'Mark watched'}
      disabled={!tracksVideoProgress || isBusy}
      onclick={() =>
        video.isWatched
          ? actions.onMarkUnwatched(video)
          : actions.onMarkWatched(video)}
      >{#if video.isWatched}<EyeOff class="size-3.5" />{:else}<Eye
          class="size-3.5"
        />{/if}</button
    >
    {#if downloadPending}
      <button
        type="button"
        class="grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-foreground"
        title="Cancel download"
        disabled={downloadBusy}
        onclick={() => actions.onCancelDownload(video)}
        ><X class="size-3.5" /></button
      >
    {:else if downloadReady}
      <button
        type="button"
        class="grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-danger"
        title="Delete download"
        disabled={downloadBusy || isPlaying}
        onclick={() => actions.onDeleteDownload(video)}
        ><Trash2 class="size-3.5" /></button
      >
    {:else}
      <button
        type="button"
        class="grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-foreground"
        title="Download"
        disabled={!canDownload}
        onclick={() => actions.onDownload(video)}
        ><Download class="size-3.5" /></button
      >
    {/if}
    <button
      type="button"
      class="grid size-7 place-items-center text-muted hover:bg-surface-strong hover:text-danger"
      title="Remove from watchlist"
      onclick={() => onRemove(video)}><X class="size-3.5" /></button
    >
  </div>
</div>
