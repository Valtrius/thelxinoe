<script lang="ts">
  import type { Snippet } from 'svelte';
  import { twMerge } from 'tailwind-merge';

  let {
    tone,
    children,
    class: className,
  }: {
    tone: string;
    children: Snippet;
    class?: string;
  } = $props();
</script>

<span
  data-tone={tone}
  class={twMerge(
    "flex items-center gap-1.25 text-[9px] leading-[1.3] text-muted before:size-1.25 before:shrink-0 before:rounded-full before:bg-current before:content-[''] data-[tone=ok]:text-success data-[tone=bad]:text-danger data-[tone=warn]:text-warning data-[tone=busy]:text-accent data-[tone=busy]:before:animate-status-pulse motion-reduce:before:animate-none",
    className,
  )}
>
  {@render children()}
</span>
