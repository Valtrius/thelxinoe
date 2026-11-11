<script lang="ts">
  import { Music } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  import { libraryArtworkSet } from '../artwork';
  import { artwork } from './library';
  let {
    url,
    class: className = '',
    sizes = '12rem',
    lazy = false,
    ...rest
  }: {
    url?: string | null;
    class?: string;
    sizes?: string;
    /** Wait until the image is close to being seen. */
    lazy?: boolean;
    'data-sidebar-resize'?: string;
  } = $props();
  let failed = $state(false);
  const source = $derived(artwork(url));
  $effect(() => {
    void source;
    failed = false;
  });
</script>

{#if source && !failed}<img
    src={source}
    srcset={libraryArtworkSet(source)}
    {sizes}
    {...rest}
    alt=""
    draggable="false"
    loading={lazy ? 'lazy' : undefined}
    class={twMerge('shrink-0 object-cover', className)}
    onerror={() => (failed = true)}
  />{:else}<div
    {...rest}
    class={twMerge(
      'grid shrink-0 place-items-center bg-[linear-gradient(130deg,#121923,#080b10_60%)] text-accent',
      className,
    )}
    aria-hidden="true"
  >
    <Music class="size-1/3" />
  </div>{/if}
