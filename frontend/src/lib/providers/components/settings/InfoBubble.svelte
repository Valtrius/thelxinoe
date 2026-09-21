<script lang="ts">
  import { onMount } from 'svelte';
  import type { Snippet } from 'svelte';
  import { Info } from '@lucide/svelte';

  let {
    children,
    label = 'More information',
  }: {
    children: Snippet;
    label?: string;
  } = $props();
  let details = $state<HTMLDetailsElement>();
  let open = $state(false);

  onMount(() => {
    const closeOutside = (event: PointerEvent) => {
      if (open && details && !details.contains(event.target as Node))
        open = false;
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') open = false;
    };
    document.addEventListener('pointerdown', closeOutside, true);
    window.addEventListener('keydown', closeOnEscape);
    return () => {
      document.removeEventListener('pointerdown', closeOutside, true);
      window.removeEventListener('keydown', closeOnEscape);
    };
  });
</script>

<details bind:this={details} bind:open class="group relative inline-block">
  <summary
    class="flex size-5 cursor-pointer list-none items-center justify-center text-muted hover:text-foreground [&::-webkit-details-marker]:hidden"
    aria-label={label}
    title={label}
  >
    <Info class="size-3.5" />
  </summary>
  <div
    class="absolute top-full left-0 z-30 mt-1 w-72 border border-line-strong bg-surface-strong p-3 text-xs leading-5 text-muted shadow-xl"
  >
    {@render children()}
  </div>
</details>
