<script lang="ts">
  import { Film, Star } from '@lucide/svelte';
  import {
    artwork,
    availability,
    title,
    year,
    kind,
    type Media,
  } from './types';
  let { item, open } = $props<{ item: Media; open: (item: Media) => void }>();
  let failed = $state(false);
  const status = $derived(availability(item.mediaInfo?.status));
</script>

<button
  class="group/poster block w-full min-w-0 text-left outline-offset-4 focus-visible:outline-2 focus-visible:outline-accent"
  onclick={() => open(item)}
  aria-label={`View ${title(item)}`}
>
  <div
    class="relative aspect-2/3 overflow-hidden border border-line bg-surface-strong transition-[border-color,translate] duration-150 group-hover/poster:-translate-y-1 group-hover/poster:border-accent motion-reduce:transition-none"
  >
    {#if artwork(item.posterPath) && !failed}<img
        class="size-full object-cover"
        src={artwork(item.posterPath)}
        alt=""
        loading="lazy"
        onerror={() => (failed = true)}
      />{:else}<div class="grid size-full place-items-center p-4 text-muted">
        <Film size={36} />
      </div>{/if}
    {#if status}<span
        class={[
          'absolute inset-x-0 bottom-0 px-2 py-2 text-center text-[10px] font-semibold backdrop-blur-md',
          item.mediaInfo?.status === 5
            ? 'bg-emerald-950/90 text-emerald-100'
            : 'bg-black/80 text-white',
        ]}>{status}</span
      >{/if}
    {#if (item.voteAverage ?? 0) > 0}<span
        class="absolute top-2 right-2 flex items-center gap-1 bg-black/80 px-1.5 py-1 text-[10px] text-white"
        ><Star size={10} class="text-amber-300" />{item.voteAverage!.toFixed(
          1,
        )}</span
      >{/if}
  </div>
  <strong
    class="mt-2.5 line-clamp-2 block text-xs leading-5 font-semibold group-hover/poster:text-accent"
    >{title(item)}</strong
  >
  <span class="mt-0.5 block text-[10px] text-muted"
    >{year(item)}{year(item) ? ' · ' : ''}{kind(item) === 'tv'
      ? 'Series'
      : 'Movie'}</span
  >
</button>
