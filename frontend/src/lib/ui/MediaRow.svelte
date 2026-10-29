<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import Button from './Button.svelte';
  import { appearance } from '../appearance';
  import { scaleCardScope } from '../card-grid-zoom';
  import { scrollWithMotion } from '../motion';
  import { compactViewport } from '../responsive';

  let { label, children } = $props<{ label: string; children: Snippet }>();
  // Home cards are sized like the video pages' cards.
  const columns = $derived(
    compactViewport.current ? 2 : $appearance.video_columns,
  );
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
    if (rail)
      scrollWithMotion(rail, {
        left: rail.scrollLeft + direction * rail.clientWidth * 0.8,
      });
  }
</script>

<section
  class="media-row min-w-0"
  aria-label={label}
  data-sidebar-resize="xy-pos"
>
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
  <div class="min-w-0" use:scaleCardScope={columns}>
    <div
      bind:this={rail}
      data-card-grid
      data-sidebar-resize="media"
      data-sidebar-group
      data-sidebar-resize-origin
      onscroll={measure}
      class="media-row-rail grid grid-flow-col items-start overflow-x-auto overscroll-x-contain pt-1 pb-4"
    >
      {@render children()}
    </div>
  </div>
</section>
