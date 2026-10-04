<script lang="ts">
  import Skeleton from './Skeleton.svelte';
  import MediaGrid from './MediaGrid.svelte';
  import MediaRow from './MediaRow.svelte';
  let {
    label,
    heading,
    layout = 'grid',
    shape = 'poster',
    count = layout === 'row' ? 6 : 12,
  } = $props<{
    label: string;
    heading?: string;
    layout?: 'grid' | 'row' | 'poster-row' | 'feed' | 'posters';
    shape?: 'poster' | 'square' | 'landscape';
    count?: number;
  }>();
</script>

{#snippet cards()}
  {#each Array.from({ length: count }, (_, index) => index) as index (index)}
    <div class="min-w-0" aria-hidden="true">
      <Skeleton
        class={shape === 'landscape'
          ? 'aspect-video h-auto'
          : shape === 'square'
            ? 'aspect-square h-auto'
            : 'aspect-2/3 h-auto'}
      />
      <Skeleton class="mt-3 w-4/5" />
      <Skeleton class="mt-2 h-2 w-1/2" />
    </div>
  {/each}
{/snippet}

<div role="status" aria-label={label} aria-busy="true" class="min-w-0">
  <span class="sr-only">{label}</span>
  {#if layout === 'row'}
    <MediaRow label={heading ?? label}>{@render cards()}</MediaRow>
  {:else if layout === 'poster-row'}
    <section class="min-w-0" aria-label={heading ?? label}>
      <h2 class="mb-4 text-base font-semibold tracking-tight">
        {heading ?? label}
      </h2>
      <div
        class="grid auto-cols-[clamp(8.5rem,13vw,11.5rem)] grid-flow-col gap-4 overflow-x-auto overscroll-x-contain pt-1 pb-4 compact:auto-cols-[8.5rem] compact:gap-3"
      >
        {@render cards()}
      </div>
    </section>
  {:else if layout === 'feed'}
    <section
      data-card-grid
      class="grid items-stretch [contain:layout_style]"
      aria-hidden="true"
    >
      {@render cards()}
    </section>
  {:else if layout === 'posters'}
    <div
      class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-2 compact:gap-3"
      aria-hidden="true"
    >
      {@render cards()}
    </div>
  {:else}
    <MediaGrid {label}>{@render cards()}</MediaGrid>
  {/if}
</div>
