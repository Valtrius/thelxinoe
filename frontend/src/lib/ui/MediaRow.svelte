<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import Button from './Button.svelte';

  let { label, children } = $props<{ label: string; children: Snippet }>();
  let rail = $state<HTMLDivElement>();
  function scroll(direction: number) {
    rail?.scrollBy({
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
      >
      <Button
        variant="ghost"
        size="sm"
        aria-label={`Next ${label}`}
        onclick={() => scroll(1)}><ChevronRight size={17} /></Button
      >
    </div>
  </div>
  <div
    bind:this={rail}
    class="grid auto-cols-[clamp(14rem,23vw,20rem)] grid-flow-col items-start gap-4 overflow-x-auto overscroll-x-contain pt-1 pb-4 compact:auto-cols-[min(78vw,18rem)] compact:gap-3"
  >
    {@render children()}
  </div>
</section>
