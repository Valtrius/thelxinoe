<script lang="ts">
  import type { Snippet } from 'svelte';
  import ProgressBar from './ProgressBar.svelte';
  import { Film, Music, Tv, Play, MoreHorizontal } from '@lucide/svelte';
  import { serverUrl } from '../api';
  import AttentionDot from './AttentionDot.svelte';
  import PlatformIcon from './PlatformIcon.svelte';
  import { attentionDescription, type AttentionItem } from '../attention';
  import type { ActivityItem } from '../activity';
  import { appearance } from '../appearance';
  import { mediaCardMotionClass, mediaCardThumbnailClass } from './styles';
  import { statusChip, type TitleStatus } from '../title/status';
  import { launchingMedia } from '../native-playback';
  import LaunchingOverlay from './LaunchingOverlay.svelte';
  import StatusChip from './StatusChip.svelte';
  import { youtubeThumbnailSet } from '../providers/utils';
  import { libraryArtworkSet } from '../artwork';
  let {
    item,
    open,
    play,
    details,
    menu,
    timezone,
    keyPrefix = 'library',
    attention = [],
    activity = [],
    onSeen,
    attentionError = '',
    landscape = false,
  } = $props<{
    item: {
      id: string;
      title: string;
      kind: string;
      year?: number | null;
      artwork_url?: string;
      backdrop_url?: string | null;
      show_title?: string;
      available?: boolean;
      position?: number;
      duration?: number;
      watched?: boolean;
      virtual?: boolean;
      status?: TitleStatus | null;
      ended?: boolean;
    };
    keyPrefix?: string;
    open: () => void;
    play?: () => void;
    details?: () => void;
    /** A title menu; library sections pass it or `details`, not both. */
    menu?: Snippet;
    timezone?: string;
    attention?: AttentionItem[];
    activity?: ActivityItem[];
    onSeen?: () => void;
    attentionError?: string;
    landscape?: boolean;
  }>();
  const music = $derived(['artist', 'album', 'track'].includes(item.kind));
  const wide = $derived(landscape || item.kind === 'episode');
  const faded = $derived(
    $appearance.fade_watched &&
      item.watched &&
      ['movie', 'show', 'season', 'episode'].includes(item.kind),
  );
  const cardId = $props.id();
  const chip = $derived(timezone ? statusChip(item.status, timezone) : null);
  const launching = $derived(launchingMedia.has(item.id));
  const chipClass = {
    success: 'text-success',
    accent: 'text-accent',
    warning: 'text-warning',
    muted: 'text-[#d7dce2]',
  };
  const description = $derived(
    attentionError || attentionDescription(attention, activity),
  );
  // Landscape tiles prefer the title's backdrop. Other art that is not landscape
  // (posters, album covers) shows whole over a blurred copy of itself instead
  // of being cropped to its middle.
  let backdropFailed = $state(false),
    imageFailed = $state(false),
    narrow = $state(false);
  const backdrop = $derived(
    wide && item.backdrop_url && !backdropFailed ? item.backdrop_url : null,
  );
  const art = $derived(backdrop ?? item.artwork_url ?? '');
  const source = $derived(
    !art ? '' : /^https?:\/\//.test(art) ? art : `${serverUrl()}${art}`,
  );
  const sourceSet = $derived(
    item.kind === 'youtube'
      ? youtubeThumbnailSet(source)
      : libraryArtworkSet(source),
  );
  $effect(() => {
    void source;
    narrow = false;
  });
  function measure(event: Event) {
    const { naturalWidth, naturalHeight } =
      event.currentTarget as HTMLImageElement;
    narrow = wide && !backdrop && naturalWidth / naturalHeight < 1.4;
  }
  const actionClass =
    'grid size-7.5 min-h-0 place-items-center gap-2 border border-[#ffffff30] bg-[#080b10c9] p-0 text-[0.68rem] font-semibold tracking-[0.08em] text-white uppercase';
</script>

<article
  class={`media-tile library-tile group/library-card group/media-card relative h-full min-w-0 border border-line bg-surface shadow-card hover:border-line-strong ${mediaCardMotionClass} ${faded ? 'opacity-55 focus-within:opacity-100 hover:opacity-100' : 'opacity-100'}`}
  data-layout-key={`${keyPrefix}:${item.id}`}
  data-sidebar-resize="xy"
>
  {#if play || details || menu}<div
      class="card-actions absolute top-2 right-2 z-2 flex gap-1 opacity-0 transition-opacity duration-(--animation-duration) group-hover/library-card:opacity-100 group-focus-within/library-card:opacity-100 compact:opacity-100"
    >
      {#if play}<button
          class={actionClass}
          aria-label={`Play ${item.title}`}
          title="Play"
          disabled={item.available === false}
          onclick={play}><Play size={15} /></button
        >{/if}{#if menu}{@render menu()}{/if}{#if details}<button
          class={actionClass}
          title="Details and metadata"
          aria-label={`Details ${item.title}`}
          onclick={details}><MoreHorizontal size={16} /></button
        >{/if}
    </div>{/if}
  <button
    class="card-primary block w-full bg-transparent p-0 text-left text-inherit"
    aria-labelledby={`${cardId}-label`}
    aria-describedby={description ? `${cardId}-description` : undefined}
    onclick={open}
    onfocus={onSeen}
  >
    <div
      class={[
        'tile-art relative grid w-full place-items-center overflow-hidden bg-[linear-gradient(130deg,#121923,#080b10_60%)] text-accent [content-visibility:auto]',
        landscape || (wide && !music)
          ? 'aspect-video'
          : music
            ? 'aspect-square'
            : 'aspect-2/3',
      ]}
      onpointerenter={onSeen}
      role="presentation"
      title={attentionError ||
        (attention.length ? 'Your requested media is available' : undefined)}
    >
      {#if source && !imageFailed}{#if narrow}<img
            class="absolute size-full scale-125 object-cover opacity-60 blur-xl"
            src={source}
            srcset={sourceSet}
            sizes="auto, 350px"
            loading="lazy"
            alt=""
            aria-hidden="true"
            data-art-backdrop
          />{/if}<img
          class={`absolute size-full ${narrow ? 'object-contain' : 'object-cover'} ${mediaCardThumbnailClass} ${item.virtual ? 'opacity-45 grayscale' : ''}`}
          src={source}
          srcset={sourceSet}
          sizes="auto, 350px"
          alt=""
          loading="lazy"
          data-art={backdrop ? 'backdrop' : narrow ? 'contained' : 'cover'}
          onload={measure}
          onerror={() =>
            backdrop ? (backdropFailed = true) : (imageFailed = true)}
        />{:else if launching}<!-- The spinner replaces the placeholder. -->{:else if item.kind === 'youtube'}<PlatformIcon
          platform="youtube"
          class="size-12"
        />{:else if music}<Music size={48} />{:else if item.kind === 'show'}<Tv
          size={48}
        />{:else}<Film size={48} />{/if}
      {#if launching}<LaunchingOverlay label={`Starting ${item.title}`} />{/if}
      {#if chip}<span
          class={`absolute bottom-2 left-2 z-2 max-w-[calc(100%-1rem)] truncate border border-[#ffffff30] bg-[#080b10e6] px-2 py-1 text-[11px] leading-none font-semibold ${chipClass[chip.tone]}`}
          data-title-chip={item.status?.state}>{chip.label}</span
        >{/if}
      {#if item.position && item.duration}<ProgressBar
          class="card-progress absolute inset-x-0 bottom-0 h-0.75 bg-transparent"
          value={(item.position / item.duration) * 100}
          barClass="shadow-accent-glow"
        />{/if}
      <AttentionDot
        items={attention}
        {activity}
        class={activity.length
          ? 'absolute top-2 left-2 z-3 bg-surface-strong/90 rounded-full'
          : 'absolute top-2 left-2 z-3 size-2.5 border-2 border-surface-strong box-content'}
      />
    </div>
    <div id={`${cardId}-label`} class="tile-body min-w-0 p-4">
      {#if item.show_title}<div
          class="mb-2 flex justify-between gap-2.5 text-[10px] tracking-[0.08em] text-muted uppercase"
        >
          {item.show_title}
        </div>{/if}
      <h3 class="text-[15px] leading-5 tracking-[-0.02em]">{item.title}</h3>
      <p class="mt-2 mb-0 text-[11px] leading-normal text-muted">
        {item.year ?? item.kind}{item.available === false &&
        !chip &&
        ['movie', 'episode', 'track'].includes(item.kind)
          ? ' · Unavailable'
          : ''}{#if item.ended}<StatusChip
            dot={false}
            class="ml-1.5 py-px align-[1px]">Ended</StatusChip
          >{/if}{#if chip || item.virtual}<span class="sr-only"
            >{item.virtual ? ' · Not in your library' : ''}{chip
              ? ` · ${chip.label}`
              : ''}</span
          >{/if}
      </p>
    </div>
  </button>
  {#if description}<span id={`${cardId}-description`} class="sr-only"
      >{description}</span
    >{/if}
</article>
