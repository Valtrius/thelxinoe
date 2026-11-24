<script lang="ts">
  import type { Snippet } from 'svelte';
  import {
    youtubeThumbnailRetry,
    youtubeThumbnailSet,
    youtubeThumbnailUrl,
  } from '../../utils';

  // A new video's images can lag behind it, and a fetch can fail in transit.
  const retryDelays = [2_000, 10_000, 30_000];

  let {
    url,
    class: className,
    sizes,
    draggable,
    onload,
    fallback,
  }: {
    url?: string | null;
    class: string;
    sizes: string;
    draggable?: 'true' | 'false';
    onload: (event: Event) => void;
    /** Shown instead of the image while it is missing. */
    fallback?: Snippet;
  } = $props();

  const source = $derived(youtubeThumbnailUrl(url));
  let retry = $state({
    source: null as string | null,
    attempt: 0,
    failed: false,
  });
  const current = $derived(
    retry.source === source ? retry : { source, attempt: 0, failed: false },
  );
  const address = $derived(
    source && youtubeThumbnailRetry(source, current.attempt),
  );

  $effect(() => {
    const { source, attempt, failed } = current;
    if (!failed || attempt >= retryDelays.length) return;
    const timer = setTimeout(
      () => (retry = { source, attempt: attempt + 1, failed: false }),
      retryDelays[attempt],
    );
    return () => clearTimeout(timer);
  });
</script>

{#if address && !current.failed}
  <img
    class={className}
    src={address}
    srcset={youtubeThumbnailSet(address)}
    {sizes}
    alt=""
    loading="lazy"
    {draggable}
    {onload}
    onerror={() => (retry = { ...current, failed: true })}
  />
{:else}
  {@render fallback?.()}
{/if}
