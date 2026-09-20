<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    tone,
    pulse = false,
    children,
  }: {
    tone: 'twitch' | 'kick' | 'neutral' | 'playing';
    pulse?: boolean;
    children: Snippet;
  } = $props();

  const tones = {
    twitch:
      'border-[color-mix(in_srgb,var(--danger)_55%,transparent)] text-[#ff8b91]',
    kick: 'border-[#53fc18]/55 text-[#72ff43]',
    neutral: 'border-white/20 text-white/75',
    playing:
      'border-[color-mix(in_srgb,var(--accent)_55%,transparent)] text-(--accent)',
  };
</script>

<span
  class={`border bg-[rgba(5,7,10,0.82)] px-[0.45rem] py-[0.28rem] text-[0.58rem] font-bold tracking-[0.1em] uppercase ${tones[tone]} ${pulse ? 'animate-pulse' : ''}`}
>
  {@render children()}
</span>
