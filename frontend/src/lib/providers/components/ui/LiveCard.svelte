<script lang="ts">
  import type { Snippet } from 'svelte';
  import { elapsedTime, formatViewers, liveNow } from '../../utils';
  import LiveCardBadge from './LiveCardBadge.svelte';

  let {
    platform,
    streamId,
    channelSlug,
    displayName,
    title,
    thumbnailUrl,
    profileImageUrl,
    statistics,
    isPlaying,
    isLaunching,
    watchDisabled,
    watchLabel,
    onWatch,
    badges,
    fallback,
    shortcuts,
    metadata,
  }: {
    platform: 'twitch' | 'kick';
    streamId?: string;
    channelSlug?: string;
    displayName: string;
    title: string;
    thumbnailUrl?: string | null;
    profileImageUrl?: string | null;
    statistics?: { viewerCount: number; startedAt: string } | null;
    isPlaying: boolean;
    isLaunching: boolean;
    watchDisabled: boolean;
    watchLabel: string;
    onWatch: () => void;
    badges: Snippet;
    fallback: Snippet;
    shortcuts: Snippet;
    metadata: Snippet;
  } = $props();

  const isBusy = $derived(isPlaying || isLaunching);
</script>

<article
  data-channel-slug={channelSlug}
  data-stream-id={streamId}
  data-playing={isPlaying}
  class={`group relative flex h-full flex-col overflow-hidden border bg-surface shadow-card transition-[border-color,transform] ${isBusy ? (platform === 'kick' ? 'border-[#53fc18]' : 'border-accent') : 'border-line hover:-translate-y-0.5 hover:border-line-strong'}`}
>
  {#if isPlaying}
    <span
      aria-hidden="true"
      class={`pointer-events-none absolute inset-0 z-20 border-3 ${platform === 'kick' ? 'border-[#53fc18]' : 'border-accent'}`}
    ></span>
  {/if}
  <div
    class="relative aspect-video w-full shrink-0 overflow-hidden bg-[#080b10]"
  >
    <button
      type="button"
      class={`absolute inset-0 size-full text-left disabled:cursor-not-allowed ${isBusy ? 'disabled:opacity-100' : ''}`}
      disabled={watchDisabled}
      aria-label={watchLabel}
      onclick={onWatch}
    >
      {#if thumbnailUrl}
        <img
          class="size-full object-cover transition-transform duration-300 group-hover:scale-102"
          src={thumbnailUrl}
          alt=""
          loading="lazy"
        />
      {:else}
        {@render fallback()}
      {/if}
      <span
        class="absolute inset-0 bg-linear-to-t from-black/60 via-transparent to-black/15"
      ></span>
    </button>
    <div class="pointer-events-none absolute top-2 left-2 flex gap-1">
      {@render badges()}
      {#if isLaunching}
        <LiveCardBadge tone="playing" pulse>Launching</LiveCardBadge>
      {:else if isPlaying}
        <LiveCardBadge tone="playing">Playing</LiveCardBadge>
      {/if}
    </div>
    <div
      class="pointer-events-none absolute top-2 right-2 z-10 flex gap-1 opacity-0 transition-opacity group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100"
    >
      {@render shortcuts()}
    </div>
    {#if statistics}
      <span
        class="absolute right-2 bottom-2 border border-white/20 bg-black/75 px-2 py-1 font-mono text-[0.62rem] text-white"
      >
        {formatViewers(statistics.viewerCount)} viewers
      </span>
      <span
        class={`absolute bottom-2 left-2 border bg-black/75 px-2 py-1 font-mono text-[0.62rem] ${platform === 'kick' ? 'border-[#53fc18]/45 text-[#72ff43]' : 'border-accent/45 text-accent'}`}
      >
        {elapsedTime(statistics.startedAt, liveNow())}
      </span>
    {/if}
  </div>

  <div class="flex flex-1 flex-col p-4">
    <div class="flex items-start gap-3">
      {#if profileImageUrl}
        <img
          class="size-9 shrink-0 rounded-full border border-line object-cover"
          src={profileImageUrl}
          alt=""
          loading="lazy"
        />
      {:else}
        <span
          class={`grid size-9 shrink-0 place-items-center rounded-full border border-line bg-surface-soft text-xs ${platform === 'kick' ? 'text-[#72ff43]' : 'text-accent'}`}
        >
          {displayName.slice(0, 1).toUpperCase()}
        </span>
      {/if}
      <div class="min-w-0">
        <h3 class="truncate text-sm font-semibold">{displayName}</h3>
        <p class="mt-1 text-xs leading-5 text-muted">{title}</p>
      </div>
    </div>
    {@render metadata()}
  </div>
</article>
