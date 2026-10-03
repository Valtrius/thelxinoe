<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import Button from './Button.svelte';

  let { label, children } = $props<{ label: string; children: Snippet }>();
  let rail = $state<HTMLDivElement>();
  let overflow = $state(false);
  let previous = $state(false);
  let next = $state(false);
  function measure() {
    if (!rail) return;
    overflow = rail.scrollWidth > rail.clientWidth + 1;
    previous = rail.scrollLeft > 1;
    next = rail.scrollLeft + rail.clientWidth < rail.scrollWidth - 1;
  }
  onMount(() => {
    if (!rail) return;
    const resize = new ResizeObserver(measure);
    const contents = new MutationObserver(measure);
    resize.observe(rail);
    contents.observe(rail, { childList: true });
    measure();
    return () => {
      resize.disconnect();
      contents.disconnect();
    };
  });
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
  <div class="mb-4 flex min-h-8 items-center justify-between gap-3">
    <h2 class="text-base font-semibold tracking-tight">{label}</h2>
    {#if overflow}<div class="flex gap-1">
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Previous ${label}`}
          disabled={!previous}
          onclick={() => scroll(-1)}><ChevronLeft size={17} /></Button
        >
        <Button
          variant="ghost"
          size="sm"
          aria-label={`Next ${label}`}
          disabled={!next}
          onclick={() => scroll(1)}><ChevronRight size={17} /></Button
        >
      </div>{/if}
  </div>
  <div
    bind:this={rail}
    onscroll={measure}
    class="grid auto-cols-[clamp(14rem,23vw,20rem)] grid-flow-col items-start gap-4 overflow-x-auto overscroll-x-contain pt-1 pb-4 compact:auto-cols-[min(78vw,18rem)] compact:gap-3"
  >
    {@render children()}
  </div>
</section>
