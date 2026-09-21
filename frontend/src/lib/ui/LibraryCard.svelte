<script lang="ts">
  import { Film, Music, Tv, Play, MoreHorizontal } from '@lucide/svelte';
  import { serverUrl } from '../api';
  let {
    item,
    open,
    play,
    details,
    keyPrefix = 'library',
  } = $props<{
    item: {
      id: string;
      title: string;
      kind: string;
      year?: number | null;
      artwork_url?: string;
      show_title?: string;
      available?: boolean;
      position?: number;
      duration?: number;
    };
    keyPrefix?: string;
    open: () => void;
    play?: () => void;
    details?: () => void;
  }>();
  const music = $derived(['artist', 'album', 'track'].includes(item.kind));
  const landscape = $derived(item.kind === 'episode');
  let imageFailed = $state(false);
  const actionClass =
    'grid size-7.5 min-h-0 place-items-center gap-2 border border-[#ffffff30] bg-[#080b10c9] p-0 text-[0.68rem] font-semibold tracking-[0.08em] text-white uppercase';
</script>

<article
  class="media-tile library-tile group/library-card relative h-full min-w-0 border border-line bg-surface shadow-card transition-[border-color,opacity,translate] duration-200 hover:-translate-y-0.5 hover:border-line-strong"
  data-layout-key={`${keyPrefix}:${item.id}`}
  data-sidebar-resize="xy"
>
  {#if play || details}<div
      class="card-actions absolute top-2 right-2 z-2 flex gap-1 opacity-0 transition-opacity duration-150 group-hover/library-card:opacity-100 group-focus-within/library-card:opacity-100 compact:opacity-100"
    >
      {#if play}<button
          class={actionClass}
          aria-label={`Play ${item.title}`}
          title="Play"
          disabled={item.available === false}
          onclick={play}><Play size={15} /></button
        >{/if}{#if details}<button
          class={actionClass}
          title="Details and metadata"
          aria-label={`Details ${item.title}`}
          onclick={details}><MoreHorizontal size={16} /></button
        >{/if}
    </div>{/if}
  <button
    class="card-primary block w-full bg-transparent p-0 text-left text-inherit"
    onclick={open}
  >
    <div
      class={[
        'tile-art relative grid w-full place-items-center overflow-hidden bg-[linear-gradient(130deg,#121923,#080b10_60%)] text-accent',
        music ? 'aspect-square' : landscape ? 'aspect-video' : 'aspect-2/3',
      ]}
    >
      {#if item.artwork_url && !imageFailed}<img
          class="absolute size-full object-cover"
          src={`${serverUrl()}${item.artwork_url}`}
          alt=""
          loading="lazy"
          onerror={() => (imageFailed = true)}
        />{:else if music}<Music size={48} />{:else if item.kind === 'show'}<Tv
          size={48}
        />{:else}<Film size={48} />{/if}
      {#if item.position && item.duration}<span
          class="card-progress absolute bottom-0 left-0 h-0.75 bg-accent shadow-accent-glow"
          style:width={`${Math.min(100, (item.position / item.duration) * 100)}%`}
        ></span>{/if}
    </div>
    <div class="tile-body min-w-0 p-4">
      {#if item.show_title}<div
          class="mb-2 flex justify-between gap-2.5 text-[10px] tracking-[0.08em] text-muted uppercase"
        >
          {item.show_title}
        </div>{/if}
      <h3 class="text-[15px] leading-5 tracking-[-0.02em]">{item.title}</h3>
      <p class="mt-2 mb-0 text-[11px] leading-normal text-muted">
        {item.year ?? item.kind}{item.available === false &&
        ['movie', 'episode', 'track'].includes(item.kind)
          ? ' · Unavailable'
          : ''}
      </p>
    </div>
  </button>
</article>
