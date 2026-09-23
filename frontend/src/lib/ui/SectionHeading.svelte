<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';

  let {
    children,
    class: className,
    ...rest
  }: Omit<HTMLAttributes<HTMLDivElement>, 'children' | 'class'> & {
    children: Snippet;
    class?: string;
  } = $props();
</script>

<div
  class={twMerge(
    'mb-4.5 flex items-center justify-between gap-4 [&_h2]:mb-0 [&_p]:mb-0 compact:flex-wrap compact:items-start',
    className,
  )}
  {...rest}
>
  {@render children()}
</div>
