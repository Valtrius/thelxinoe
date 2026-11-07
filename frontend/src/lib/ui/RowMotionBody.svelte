<script lang="ts">
  import { onDestroy, type Snippet } from 'svelte';
  import type { ClassValue } from 'svelte/elements';
  import ProgressBar from './ProgressBar.svelte';
  import type { RowMotion } from './row-motion.svelte';

  // `loaded` is whether there are rows yet: before that, a skeleton shows the
  // wait instead of dimmed rows.
  let {
    motion,
    loaded = true,
    class: className,
    children,
  }: {
    motion: RowMotion;
    loaded?: boolean;
    class?: ClassValue;
    children: Snippet;
  } = $props();
  const dimmed = $derived(motion.stale && loaded);
  onDestroy(() => motion.release());
</script>

<div
  bind:this={motion.root}
  class="relative min-w-0"
  aria-busy={motion.holding}
>
  {#if dimmed}
    <ProgressBar
      indeterminate
      class="absolute inset-x-0 top-0 z-10 h-0.5 bg-transparent"
    />
  {/if}
  <div
    bind:this={motion.content}
    class={[
      'min-w-0 transition-opacity motion-reduce:transition-none',
      dimmed && 'opacity-55',
      className,
    ]}
  >
    {@render children()}
  </div>
</div>
