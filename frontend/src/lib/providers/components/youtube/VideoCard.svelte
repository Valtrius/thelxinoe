<script lang="ts">
  import VideoActionsMenu from './VideoActionsMenu.svelte';
  import type { YoutubeVideoActions } from '../../youtube-video-actions';

  import { youtubeVideoPresentation } from '../../youtube-video-presentation';

  import { Download } from '@lucide/svelte';
  import type {
    YoutubeCardShortcut,
    YoutubeDownloadEvent,
    YoutubeWatchlist,
    YoutubeVideo,
  } from '../../types';
  import {
    formatBytes,
    formatClock,
    relativeTime,
    youtubeThumbnailUrl,
  } from '../../utils';

  let {
    actions,
    video,
    mpvReady,
    ytdlpReady,
    ffmpegReady,
    fadeWatchedCards,
    youtubeCardShortcuts,
    signedInPlaybackAvailable,
    isPlaying,
    isLaunching,
    downloadBusy,
    downloadEvent,
    watchlists,
    menuOpen,
    shortcutDeleteConfirming,
    onMenuToggle,
    onShortcutDeleteRequest,
    onShortcutDeleteCancel,
  }: {
    actions: YoutubeVideoActions;
    video: YoutubeVideo;
    mpvReady: boolean;
    ytdlpReady: boolean;
    ffmpegReady: boolean;
    fadeWatchedCards: boolean;
    youtubeCardShortcuts: YoutubeCardShortcut[];
    signedInPlaybackAvailable: boolean;
    isPlaying: boolean;
    isLaunching: boolean;
    downloadBusy: boolean;
    downloadEvent?: YoutubeDownloadEvent;
    watchlists: YoutubeWatchlist[];
    menuOpen: boolean;
    shortcutDeleteConfirming: boolean;
    onMenuToggle: () => void;
    onShortcutDeleteRequest: () => void;
    onShortcutDeleteCancel: () => void;
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

  const isBusy = $derived(presentation.isBusy);
  const tracksVideoProgress = $derived(presentation.tracksVideoProgress);

  const showPosition = $derived(presentation.showPosition);
  const remainingSeconds = $derived(presentation.remainingSeconds);
  const primaryLabel = $derived(presentation.primaryLabel);
  const downloadPercent = $derived(presentation.downloadPercent);
  const downloadLabel = $derived(presentation.downloadLabel);
  const downloadFormatLabel = $derived(presentation.downloadFormatLabel);
  const readyDownloadTitle = $derived(presentation.readyDownloadTitle);
  const thumbnailUrl = $derived(youtubeThumbnailUrl(video.thumbnailUrl));

  let thumbnailAspectRatio = $state(16 / 9);

  const signalBadgeClass =
    'border border-(--line-strong) bg-[rgba(5,7,10,0.78)] px-[0.45rem] py-[0.28rem] text-[0.58rem] font-bold tracking-[0.1em] text-(--accent) uppercase';
  const downloadBadgeClass =
    'border border-[color-mix(in_srgb,var(--success)_55%,var(--line-strong))] bg-[color-mix(in_srgb,var(--success)_12%,rgba(5,7,10,0.9))] px-[0.45rem] py-[0.28rem] text-[0.58rem] font-bold tracking-[0.1em] text-(--success) uppercase shadow-[0_0_14px_color-mix(in_srgb,var(--success)_18%,transparent)]';

  function updateThumbnailRatio(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    if (image.naturalWidth > 0 && image.naturalHeight > 0) {
      thumbnailAspectRatio = image.naturalWidth / image.naturalHeight;
    }
  }
</script>

<article
  data-video-id={video.videoId}
  data-playing={isPlaying}
  class={`panel group relative flex h-full flex-col overflow-hidden border bg-(--surface) shadow-[0_16px_40px_var(--shadow)] transition-[border-color,opacity,transform] ${fadeWatchedCards && video.isWatched && !isBusy ? 'opacity-55 focus-within:opacity-100 hover:opacity-100' : 'opacity-100'} ${menuOpen || shortcutDeleteConfirming ? 'z-50' : ''} ${isBusy ? 'border-(--accent)' : menuOpen || shortcutDeleteConfirming ? 'border-(--line-strong)' : 'border-(--line) hover:-translate-y-0.5 hover:border-(--line-strong)'}`}
>
  {#if isPlaying}
    <span
      aria-hidden="true"
      class="pointer-events-none absolute inset-0 z-20 border-3 border-(--accent)"
    ></span>
  {/if}
  <VideoActionsMenu
    {presentation}
    {watchlists}
    {video}
    {onMenuToggle}
    {youtubeCardShortcuts}
    {actions}
    {onShortcutDeleteRequest}
    {onShortcutDeleteCancel}
    {signedInPlaybackAvailable}
    {downloadBusy}
    {isPlaying}
    {menuOpen}
    {shortcutDeleteConfirming}
    {ytdlpReady}
    {ffmpegReady}
  />

  <button
    type="button"
    disabled={!canPlay}
    class={`relative w-full shrink-0 overflow-hidden bg-[linear-gradient(130deg,#121923,#080b10_60%)] text-left ${isBusy ? 'disabled:opacity-100' : ''}`}
    style={`aspect-ratio:${thumbnailAspectRatio}`}
    aria-label={`${primaryLabel}: ${video.title}`}
    onclick={() => actions.onPlay(video)}
  >
    {#if thumbnailUrl}
      <img
        class="size-full object-contain"
        src={thumbnailUrl}
        alt=""
        loading="lazy"
        onload={updateThumbnailRatio}
      />
    {:else}
      <span
        class="absolute inset-0 bg-[repeating-linear-gradient(135deg,transparent_0_18px,color-mix(in_srgb,var(--accent)_7%,transparent)_18px_19px)]"
      ></span>
      <span class="absolute bottom-4 left-4 font-mono text-4xl text-white/10"
        >{video.videoId.slice(0, 4).toUpperCase()}</span
      >
    {/if}
    <span
      class="absolute inset-0 bg-linear-to-t from-black/45 via-transparent to-transparent"
    ></span>
    {#if tracksVideoProgress && video.durationSeconds}
      <span
        class="absolute right-2 bottom-3 border border-white/20 bg-black/75 px-2 py-1 font-mono text-[0.62rem] text-white"
      >
        {isPlaying && remainingSeconds !== null
          ? `-${formatClock(remainingSeconds)}`
          : formatClock(video.durationSeconds)}
      </span>
    {/if}
    {#if showPosition}
      <span
        class="absolute bottom-3 left-2 border border-[color-mix(in_srgb,var(--accent)_45%,transparent)] bg-black/75 px-2 py-1 font-mono text-[0.62rem] text-(--accent)"
      >
        {formatClock(video.positionSeconds)}
      </span>
    {/if}
    {#if isLaunching}<span
        class={`${signalBadgeClass} absolute top-2 left-2 animate-pulse`}
        >Launching</span
      >{:else if isPlaying}<span
        class={`${signalBadgeClass} absolute top-2 left-2`}>Playing</span
      >{:else if video.isLive}<span
        class={`${signalBadgeClass} absolute top-2 left-2`}>Live</span
      >{:else if video.isUpcoming}<span
        class={`${signalBadgeClass} absolute top-2 left-2`}>Upcoming</span
      >{:else if video.isLiveReplay}<span
        class={`${signalBadgeClass} absolute top-2 left-2`}>Replay</span
      >{/if}
    {#if downloadReady && activeDownload}<span
        class={`${downloadBadgeClass} absolute left-2 flex flex-col items-start gap-0.5 ${isLaunching || isPlaying || video.isLive || video.isUpcoming || video.isLiveReplay ? 'top-10' : 'top-2'}`}
        title={readyDownloadTitle ?? undefined}
      >
        <span class="flex items-center gap-1"
          ><Download class="size-3" />{formatBytes(
            activeDownload.fileSizeBytes,
          )}</span
        >
        {#if downloadFormatLabel}<span>{downloadFormatLabel}</span>{/if}
      </span>{:else if downloadLabel}<span
        class={`${downloadBadgeClass} absolute left-2 ${isLaunching || isPlaying || video.isLive || video.isUpcoming || video.isLiveReplay ? 'top-10' : 'top-2'}`}
        title={activeDownload?.errorMessage ?? downloadLabel}
        >{downloadLabel}</span
      >{/if}
    {#if downloadPending && downloadPercent !== null}<span
        class="absolute right-0 bottom-1 left-0 h-1 bg-[color-mix(in_srgb,var(--success)_18%,transparent)]"
        ><i
          class="block h-full bg-(--success) shadow-[0_0_12px_var(--success)]"
          style={`width:${downloadPercent}%`}
        ></i></span
      >{/if}
    {#if tracksVideoProgress && video.watchedPercentage > 0}<span
        class="absolute right-0 bottom-0 left-0 h-1 bg-white/12"
      >
        <i
          class="block h-full bg-(--accent) shadow-[0_0_12px_var(--accent)]"
          style={`width:${video.watchedPercentage}%`}
        ></i>
      </span>{/if}
  </button>

  <div class="flex flex-1 items-start gap-3 p-4 pb-5">
    {#if video.channelThumbnailUrl}
      <img
        class="size-9 shrink-0 rounded-full border border-(--line) object-cover"
        src={video.channelThumbnailUrl}
        alt=""
        loading="lazy"
      />
    {:else}
      <span
        class="grid size-9 shrink-0 place-items-center rounded-full border border-(--line) bg-(--surface-soft) text-xs text-(--accent)"
      >
        {video.channelName.slice(0, 1).toUpperCase()}
      </span>
    {/if}
    <div class="min-w-0 flex-1">
      <div
        class="mb-2 flex items-center justify-between gap-3 text-[0.62rem] tracking-[0.08em] text-(--muted) uppercase"
      >
        <span class="min-w-0 flex-1 truncate">{video.channelName}</span><time
          class="shrink-0 whitespace-nowrap"
          datetime={video.actualStartAt ??
            video.scheduledStartAt ??
            video.publishedAt}
          >{video.isLive
            ? 'Live now'
            : video.isUpcoming && video.scheduledStartAt
              ? relativeTime(video.scheduledStartAt)
              : relativeTime(video.publishedAt)}</time
        >
      </div>
      <h3 class="text-[0.94rem] leading-5 font-medium tracking-[-0.02em]">
        {video.title}
      </h3>
    </div>
  </div>
</article>
