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
</script>

<article
  class="media-tile library-tile"
  data-layout-key={`${keyPrefix}:${item.id}`}
  data-sidebar-resize="xy"
>
  {#if play || details}<div class="card-actions">
      {#if play}<button
          aria-label={`Play ${item.title}`}
          title="Play"
          disabled={item.available === false}
          onclick={play}><Play size={15} /></button
        >{/if}{#if details}<button
          title="Details and metadata"
          aria-label={`Details ${item.title}`}
          onclick={details}><MoreHorizontal size={16} /></button
        >{/if}
    </div>{/if}
  <button class="card-primary" onclick={open}>
    <div
      class="tile-art"
      class:poster={!music && !landscape}
      class:square={music}
    >
      {#if item.artwork_url && !imageFailed}<img
          src={`${serverUrl()}${item.artwork_url}`}
          alt=""
          loading="lazy"
          onerror={() => (imageFailed = true)}
        />{:else if music}<Music size={48} />{:else if item.kind === 'show'}<Tv
          size={48}
        />{:else}<Film size={48} />{/if}
      {#if item.position && item.duration}<span
          class="card-progress"
          style:width={`${Math.min(100, (item.position / item.duration) * 100)}%`}
        ></span>{/if}
    </div>
    <div class="tile-body">
      {#if item.show_title}<div class="card-overline">
          {item.show_title}
        </div>{/if}
      <h3>{item.title}</h3>
      <p>
        {item.year ?? item.kind}{item.available === false &&
        ['movie', 'episode', 'track'].includes(item.kind)
          ? ' · Unavailable'
          : ''}
      </p>
    </div>
  </button>
</article>
