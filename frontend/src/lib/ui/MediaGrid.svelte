<script lang="ts">
  import { onMount, tick, type Snippet } from 'svelte';
  import { appearance } from '../appearance';
  import {
    provideCardGridColumns,
    scaleCardScope,
    type CardKind,
  } from '../card-grid-zoom';
  import { createLayoutMotion } from '../layout-motion';
  import { compactViewport } from '../responsive';
  let {
    children,
    revision = '',
    label = 'Media collection',
    kind = 'poster',
    class: className = '',
  }: {
    children: Snippet;
    revision?: string;
    label?: string;
    kind?: CardKind;
    class?: string;
  } = $props();
  let root: HTMLDivElement;
  let width = $state(1200);
  const columns = $derived(
    compactViewport.current
      ? kind === 'poster'
        ? 3
        : 2
      : width < 640
        ? Math.max(1, Math.min(3, Math.floor(width / 150)))
        : kind === 'poster'
          ? $appearance.poster_columns
          : $appearance.video_columns,
  );
  provideCardGridColumns(() => columns);
  const motion = createLayoutMotion();
  $effect.pre(() => {
    void revision;
    const snapshot = motion.capture();
    void tick().then(() => motion.play(snapshot));
  });
  onMount(() => {
    const resize = new ResizeObserver(() => (width = root.clientWidth));
    resize.observe(root);
    width = root.clientWidth;
    return () => {
      resize.disconnect();
    };
  });
</script>

<div
  bind:this={root}
  class="media-scope w-full min-w-0"
  use:scaleCardScope={{ kind, columns }}
  use:motion.connect
>
  <div
    class={`media-card-grid mb-6 grid grid-cols-6 items-stretch gap-3 [contain:layout_style] ${className}`}
    data-card-grid
    data-sidebar-resize="media"
    data-sidebar-group
    aria-label={label}
  >
    {@render children()}
  </div>
</div>
