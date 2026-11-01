<script lang="ts" module>
  /** The rendered width of a card in a row, for picking its image size. */
  export const rowCardSizes =
    '(max-width: 1046px) 136px, (max-width: 1415px) 13vw, 184px';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronLeft, ChevronRight } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { scrollWithMotion } from '../motion';
  let { label, children } = $props<{ label: string; children: Snippet }>();
  let rail: HTMLDivElement;
  function scroll(direction: number) {
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
    data-sidebar-resize="width"
    data-sidebar-group
    class="media-row-rail grid auto-cols-[clamp(8.5rem,13vw,11.5rem)] grid-flow-col gap-4 overflow-x-auto overscroll-x-contain pt-1 pb-4 compact:auto-cols-[8.5rem] compact:gap-3"
  >
    {@render children()}
  </div>
</section>
