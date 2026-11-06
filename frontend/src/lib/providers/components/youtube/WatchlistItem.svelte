<script lang="ts">
  import ProgressBar from '../../../ui/ProgressBar.svelte';
  import LaunchingOverlay from '../../../ui/LaunchingOverlay.svelte';
  import ListSidebarCard from '../../../ui/ListSidebarCard.svelte';
  import { touchDevice } from '../../../responsive';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { youtubeVideoPresentation } from '../../youtube-video-presentation';
  import { withNativePosition } from '../../../native-playback';
  import { Download, Eye, EyeOff, Trash2, X } from '@lucide/svelte';
  import type { YoutubeDownloadEvent, YoutubeVideo } from '../../types';
  import {
    formatClock,
    youtubeThumbnailUrl,
    youtubeThumbnailSet,
  } from '../../utils';

  let {
    actions: videoActions,
    video,
    downloadEvent,
    downloadBusy,
    isPlaying = false,
    isLaunching = false,
    playerReady,
    downloadsEnabled,
    sortable = false,
    dragging = false,
    suppressClick = false,
    onRemove,
    onReorderStart,
  }: {
    actions: YoutubeVideoActions;
    video: YoutubeVideo;
    downloadEvent?: YoutubeDownloadEvent;
    downloadBusy: boolean;
    isPlaying?: boolean;
    isLaunching?: boolean;
    playerReady: boolean;
    downloadsEnabled: boolean;
    sortable?: boolean;
    dragging?: boolean;
    suppressClick?: boolean;
    onRemove: (video: YoutubeVideo) => void;
    onReorderStart?: (event: PointerEvent) => void;
  } = $props();

  const current = $derived(withNativePosition(video));
  const presentation = $derived(
    youtubeVideoPresentation(current, {
      playerReady,
      downloadsEnabled,
      isPlaying,
      isLaunching,
      downloadBusy,
      downloadEvent,
    }),
  );
  const activeDownload = $derived(presentation.activeDownload);
  const downloadReady = $derived(presentation.downloadReady);
  const downloadPending = $derived(presentation.downloadPending);
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
        ? (presentation.downloadFormatLabel ?? 'Downloaded')
        : null),
  );
  let thumbnailAspectRatio = $state(16 / 9);

  function updateThumbnailRatio(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    if (image.naturalWidth > 0 && image.naturalHeight > 0) {
      thumbnailAspectRatio = image.naturalWidth / image.naturalHeight;
    }
  }
</script>

<ListSidebarCard
  title={video.title}
  canPlay={presentation.canPlay}
  {isPlaying}
  {sortable}
  {dragging}
  {suppressClick}
  closeActionsLabel="Close watchlist video actions"
  onPlay={() => videoActions.onPlay(video)}
  {onReorderStart}
>
  {#snippet thumbnail()}
    <div
      class="relative w-[min(10rem,42%)] shrink-0 overflow-hidden bg-[linear-gradient(130deg,#121923,#080b10_60%)]"
      style={`aspect-ratio:${thumbnailAspectRatio}`}
    >
      {#if video.thumbnailUrl}
        <img
          class="size-full object-contain"
          src={youtubeThumbnailUrl(video.thumbnailUrl)}
          srcset={youtubeThumbnailSet(video.thumbnailUrl)}
          sizes="auto, 160px"
          loading="lazy"
          alt=""
          draggable="false"
          onload={updateThumbnailRatio}
        />
      {/if}
      {#if isLaunching}<LaunchingOverlay
          label={`Starting ${video.title}`}
        />{/if}
      {#if showPosition}
        <span
          class="absolute bottom-2 left-1 border border-accent/45 bg-black/75 px-1.5 py-0.5 font-mono text-[0.6rem] text-accent"
        >
          {formatClock(current.positionSeconds)}
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
  {/snippet}
  {#snippet details()}
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
  {/snippet}
  {#snippet actions({
    actionClass,
    close,
  }: {
    actionClass: string;
    close: () => void;
  })}
    {@const run = (action: (video: YoutubeVideo) => void) => {
      close();
      action(video);
    }}
    <button
      type="button"
      class={actionClass}
      title={video.isWatched ? 'Mark unwatched' : 'Mark watched'}
      disabled={!tracksVideoProgress || isBusy}
      onclick={() =>
        run(
          video.isWatched
            ? videoActions.onMarkUnwatched
            : videoActions.onMarkWatched,
        )}
      >{#if video.isWatched}<EyeOff class="size-3.5" />{:else}<Eye
          class="size-3.5"
        />{/if}{#if touchDevice.current}<span
          >{video.isWatched ? 'Mark unwatched' : 'Mark watched'}</span
        >{/if}</button
    >
    {#if downloadPending}
      <button
        type="button"
        class={actionClass}
        title="Cancel download"
        disabled={downloadBusy}
        onclick={() => run(videoActions.onCancelDownload)}
        ><X class="size-3.5" />{#if touchDevice.current}<span
            >Cancel download</span
          >{/if}</button
      >
    {:else if downloadReady}
      <button
        type="button"
        class={`${actionClass} hover:text-danger`}
        title="Delete download"
        disabled={downloadBusy || isPlaying}
        onclick={() => run(videoActions.onDeleteDownload)}
        ><Trash2 class="size-3.5" />{#if touchDevice.current}<span
            >Delete download</span
          >{/if}</button
      >
    {:else}
      <button
        type="button"
        class={actionClass}
        title="Download"
        disabled={!canDownload}
        onclick={() => run(videoActions.onDownload)}
        ><Download class="size-3.5" />{#if touchDevice.current}<span
            >Download</span
          >{/if}</button
      >
    {/if}
    <button
      type="button"
      class={`${actionClass} hover:text-danger`}
      title="Remove from watchlist"
      onclick={() => run(onRemove)}
      ><X class="size-3.5" />{#if touchDevice.current}<span
          >Remove from watchlist</span
        >{/if}</button
    >
  {/snippet}
</ListSidebarCard>
