<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import type { Snippet } from 'svelte';
  import { twMerge } from 'tailwind-merge';
  import { compactViewport } from '../responsive';
  let {
    children,
    class: className = '',
  }: {
    children: Snippet;
    class?: string;
  } = $props();
</script>

<!-- A phone has no room beside the menu, so a submenu opens below its item. -->
<DropdownMenu.SubContent
  side={compactViewport.current ? 'bottom' : 'right'}
  collisionPadding={8}
  class={twMerge(
    'z-50 grid max-h-(--bits-floating-available-height) max-w-[calc(100vw-1rem)] grid-cols-[minmax(0,1fr)] overflow-y-auto border border-line-strong bg-surface-strong p-1.5 shadow-2xl',
    className,
  )}
>
  {@render children()}
</DropdownMenu.SubContent>
