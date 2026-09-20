<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  let { title, children } = $props<{ title: string; children: Snippet }>();
  let marker: HTMLDivElement;
  let stuck = $state(false);
  onMount(() => {
    const observer = new IntersectionObserver(
      ([entry]) =>
        (stuck =
          !entry.isIntersecting &&
          entry.boundingClientRect.top < (entry.rootBounds?.top ?? 0)),
      { root: marker.closest('.workspace-scroll'), threshold: 0 },
    );
    observer.observe(marker);
    return () => observer.disconnect();
  });
</script>

<section class="feed-group">
  <div bind:this={marker} class="group-marker"></div>
  <header
    class="feed-group-header"
    class:stuck
    data-card-group-header
    data-sidebar-resize="x"
  >
    <span class="feed-group-label" data-sidebar-resize="x-pos">{title}</span
    ><span class="group-rule"></span>
  </header>
  <div class="media-card-grid" data-card-grid>{@render children()}</div>
</section>

<style>
  .feed-group {
    position: relative;
  }
  .group-marker {
    height: 1px;
  }
  .feed-group-header {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 0;
    margin-bottom: 12px;
    background: transparent;
  }
  .feed-group-header.stuck {
    background: var(--background);
  }
  .feed-group-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--muted);
    white-space: nowrap;
  }
  .group-rule {
    flex: 1;
    height: 1px;
    background: var(--line);
  }
</style>
