<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  type Platform = 'youtube' | 'twitch' | 'kick';
  type Props = Omit<HTMLButtonAttributes, 'class'> & {
    platform: Platform;
    children: Snippet;
    element?: HTMLButtonElement | null;
    class?: string;
  };

  let {
    platform,
    children,
    element = $bindable(null),
    class: className = '',
    type = 'button',
    ...rest
  }: Props = $props();

  const platformClass: Record<Platform, string> = {
    youtube:
      'hover:border-[#ff747b]/55 hover:text-[#ff747b] focus-visible:border-[#ff747b]/55 focus-visible:text-[#ff747b]',
    twitch:
      'hover:border-[#b9a4ff]/55 hover:text-[#b9a4ff] focus-visible:border-[#b9a4ff]/55 focus-visible:text-[#b9a4ff]',
    kick: 'hover:border-[#53fc18]/55 hover:text-[#72ff43] focus-visible:border-[#53fc18]/55 focus-visible:text-[#72ff43]',
  };
</script>

<button
  bind:this={element}
  {type}
  class={`grid size-8 place-items-center border border-line-strong bg-background/82 text-muted [backdrop-filter:blur(5px)] disabled:cursor-not-allowed disabled:opacity-45 [&_svg]:size-4 ${platformClass[platform]} ${className}`}
  {...rest}
>
  {@render children()}
</button>
