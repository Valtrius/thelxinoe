<script lang="ts">
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import PosterCard from './PosterCard.svelte';
  import type { Media } from './types';
  let { label, items, open } = $props<{
    label: string;
    items: Media[];
    open: (item: Media) => void;
  }>();
  let rail: HTMLDivElement;
  function scroll(direction: number) {
    rail.scrollBy({
      left: direction * rail.clientWidth * 0.8,
      behavior: matchMedia('(prefers-reduced-motion: reduce)').matches
        ? 'instant'
        : 'smooth',
    });
  }
</script>

<section class="min-w-0" aria-label={label}>
  <div class="mb-4 flex items-center justify-between gap-3">
    <h2 class="text-base font-semibold tracking-tight">{label}</h2>
    <div class="flex gap-1">
      <Button
        variant="ghost"
        size="sm"
        aria-label={`Previous ${label}`}
        onclick={() => scroll(-1)}><ChevronLeft size={17} /></Button
      ><Button
        variant="ghost"
        size="sm"
        aria-label={`Next ${label}`}
        onclick={() => scroll(1)}><ChevronRight size={17} /></Button
      >
    </div>
  </div>
  <div
    bind:this={rail}
    class="grid auto-cols-[clamp(8.5rem,13vw,11.5rem)] grid-flow-col gap-4 overflow-x-auto overscroll-x-contain pt-1 pb-4 compact:auto-cols-[8.5rem] compact:gap-3"
  >
    {#each items as item (`${item.mediaType}:${item.id}`)}<PosterCard
        {item}
        {open}
      />{/each}
  </div>
</section>
