<script lang="ts">
  import { desktop } from '../../../api';
  import { Copy, ExternalLink, Play } from '@lucide/svelte';
  import type { KickChannel } from '../../types';
  import LiveCard from '../ui/LiveCard.svelte';
  import LiveCardBadge from '../ui/LiveCardBadge.svelte';
  import CardShortcutButton from '../ui/CardShortcutButton.svelte';

  let {
    channel,
    playbackReady,
    offlineConfirmed,
    isPlaying,
    isLaunching,
    onWatch,
    onCopy,
    onOpen,
  }: {
    channel: KickChannel;
    playbackReady: boolean;
    offlineConfirmed: boolean;
    isPlaying: boolean;
    isLaunching: boolean;
    onWatch: (channel: KickChannel) => void;
    onCopy: (channel: KickChannel) => void;
    onOpen: (channel: KickChannel) => void;
  } = $props();

  const stream = $derived(channel.liveStream);
</script>

<LiveCard
  platform="kick"
  channelSlug={channel.slug}
  streamId={stream?.streamId}
  displayName={channel.displayName}
  title={stream?.title ?? `kick.com/${channel.slug}`}
  thumbnailUrl={stream?.thumbnailUrl}
  profileImageUrl={channel.profilePictureUrl}
  statistics={stream}
  {isPlaying}
  {isLaunching}
  watchDisabled={!playbackReady || offlineConfirmed || isPlaying || isLaunching}
  watchLabel={isLaunching
    ? `${channel.displayName} is launching`
    : isPlaying
      ? `${channel.displayName} is already playing`
      : offlineConfirmed
        ? `${channel.displayName} is offline`
        : `Watch ${channel.displayName}${desktop ? ' in MPV' : ''}`}
  onWatch={() => onWatch(channel)}
>
  {#snippet badges()}
    <LiveCardBadge tone={stream ? 'kick' : 'neutral'}>
      {stream ? 'Live' : offlineConfirmed ? 'Offline' : 'Tracked'}
    </LiveCardBadge>
    {#if stream?.hasMatureContent}<LiveCardBadge tone="kick">18+</LiveCardBadge
      >{/if}
  {/snippet}
  {#snippet fallback()}
    <span
      class="absolute inset-0 bg-[repeating-linear-gradient(135deg,transparent_0_18px,rgba(83,252,24,0.08)_18px_19px)]"
    ></span>
    <span
      class="absolute inset-0 grid place-items-center text-[0.62rem] font-semibold tracking-[0.08em] text-white/45 uppercase"
    >
      <span class="grid justify-items-center gap-2">
        {#if offlineConfirmed}Offline{:else}<Play class="size-8" />Watch{desktop
            ? ' in MPV'
            : ''}{/if}
      </span>
    </span>
  {/snippet}
  {#snippet shortcuts()}
    <CardShortcutButton
      platform="kick"
      aria-label={`Copy ${channel.displayName} URL`}
      title="Copy Kick URL"
      onclick={() => onCopy(channel)}
    >
      <Copy class="size-3.5" />
    </CardShortcutButton>
    <CardShortcutButton
      platform="kick"
      aria-label={`Open ${channel.displayName} in browser`}
      title="Open in browser"
      onclick={() => onOpen(channel)}
    >
      <ExternalLink class="size-3.5" />
    </CardShortcutButton>
  {/snippet}
  {#snippet metadata()}
    {#if stream}
      <div
        class="mt-3 flex flex-wrap gap-x-2 gap-y-1 text-[0.62rem] tracking-[0.06em] text-muted uppercase"
      >
        <span>{stream.categoryName ?? 'Uncategorised'}</span>
        {#if stream.languageCode}<span>· {stream.languageCode}</span>{/if}
        {#each stream.tags.slice(0, 3) as tag (tag)}<span>· #{tag}</span>{/each}
      </div>
    {/if}
  {/snippet}
</LiveCard>
