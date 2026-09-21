<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import { twMerge } from 'tailwind-merge';
  import { panelClass } from './styles';

  let {
    children,
    class: className,
    ...rest
  }: Omit<HTMLAttributes<HTMLElement>, 'children' | 'class'> & {
    children: Snippet;
    class?: string;
  } = $props();
</script>

<section class={twMerge(panelClass, className)} {...rest}>
  {@render children()}
</section>
