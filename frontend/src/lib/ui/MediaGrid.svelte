<script lang="ts">
  import { onMount, tick, type Snippet } from 'svelte';
  import { appearance } from '../appearance';
  import { scaleCardScope } from '../card-grid-zoom';
  import { createLayoutMotion } from '../layout-motion';
  let {
    children,
    revision = '',
    label = 'Media collection',
    class: className = '',
  } = $props<{
    children: Snippet;
    revision?: string;
    label?: string;
    class?: string;
  }>();
  let root: HTMLDivElement;
  let width = $state(1200);
  const columns = $derived(
    width < 640
      ? Math.max(1, Math.min(3, Math.floor(width / 150)))
      : $appearance.card_columns,
  );
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
  use:scaleCardScope={columns}
  use:motion.connect
>
  <div
    class={`media-card-grid mb-6 grid grid-cols-6 items-stretch gap-3 [contain:layout_style] ${className}`}
    data-card-grid
    aria-label={label}
  >
    {@render children()}
  </div>
</div>
