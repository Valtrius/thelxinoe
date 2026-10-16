<script lang="ts">
  import type { Snippet } from 'svelte';
  import { twMerge } from 'tailwind-merge';

  // Compact state label: a tinted dot plus uppercase text.
  let {
    tone = 'neutral',
    dot = true,
    children,
    class: className,
  }: {
    tone?: 'neutral' | 'ok' | 'warn' | 'bad' | 'info';
    dot?: boolean;
    children: Snippet;
    class?: string;
  } = $props();
</script>

<span
  data-tone={tone}
  class={twMerge(
    'inline-flex items-center gap-1.5 border border-line bg-surface-soft px-1.75 py-0.5 text-[9.5px] font-semibold tracking-[0.06em] whitespace-nowrap text-muted uppercase data-[tone=bad]:border-danger/35 data-[tone=bad]:bg-danger/9 data-[tone=bad]:text-danger data-[tone=info]:border-line-strong data-[tone=info]:bg-accent-soft data-[tone=info]:text-accent data-[tone=ok]:border-success/35 data-[tone=ok]:bg-success/10 data-[tone=ok]:text-success data-[tone=warn]:border-warning/35 data-[tone=warn]:bg-warning/8 data-[tone=warn]:text-warning',
    dot &&
      "before:size-1.5 before:shrink-0 before:rounded-full before:bg-current before:content-['']",
    className,
  )}
>
  {@render children()}
</span>
