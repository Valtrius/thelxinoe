<script lang="ts">
  import LoadingSpinner from './LoadingSpinner.svelte';
  let { surface, viewport } = $props<{
    surface: HTMLElement;
    viewport: HTMLElement;
  }>();
  let area = $state({ left: 0, top: 0, width: 0, height: 0 });
  // Center on the visible part of the loading area, below anything that stays
  // above it such as the player. When too little of it shows, use the viewport.
  $effect(() => {
    const measure = () => {
      const page = surface.getBoundingClientRect();
      const visible = viewport.getBoundingClientRect();
      const left = Math.max(page.left, visible.left);
      const top = Math.max(page.top, visible.top);
      const width = Math.min(page.right, visible.right) - left;
      const height = visible.bottom - top;
      area =
        width > 64 && height > 64
          ? { left, top, width, height }
          : {
              left: visible.left,
              top: visible.top,
              width: visible.width,
              height: visible.height,
            };
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(surface);
    observer.observe(viewport);
    return () => observer.disconnect();
  });
</script>

<div
  class="pointer-events-none fixed z-10 flex items-center justify-center transition-opacity starting:opacity-0"
  style:left={`${area.left}px`}
  style:top={`${area.top}px`}
  style:width={`${area.width}px`}
  style:height={`${area.height}px`}
>
  <LoadingSpinner label="Loading page" size={28} />
</div>
