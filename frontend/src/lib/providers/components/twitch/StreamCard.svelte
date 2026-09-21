<script lang="ts">
  import { desktop } from '../../../api';
  import { Copy, ExternalLink, MessageSquareText, Radio } from '@lucide/svelte';
  import type { TwitchLiveStream } from '../../types';
  import LiveCard from '../ui/LiveCard.svelte';
  import LiveCardBadge from '../ui/LiveCardBadge.svelte';
  import CardShortcutButton from '../ui/CardShortcutButton.svelte';

  let {
    stream,
    playbackReady,
    isPlaying,
    isLaunching,
    onWatch,
    onCopy,
    onOpen,
    onChat,
  }: {
    stream: TwitchLiveStream;
    playbackReady: boolean;
    isPlaying: boolean;
    isLaunching: boolean;
    onWatch: (stream: TwitchLiveStream) => void;
    onCopy: (stream: TwitchLiveStream) => void;
    onOpen: (stream: TwitchLiveStream) => void;
    onChat: (stream: TwitchLiveStream) => void;
  } = $props();

  const thumbnail = $derived(
    stream.thumbnailUrl?.replace('{width}', '640').replace('{height}', '360'),
  );
</script>

<LiveCard
  platform="twitch"
  streamId={stream.streamId}
  displayName={stream.displayName}
  title={stream.title}
  thumbnailUrl={thumbnail}
  profileImageUrl={stream.profileImageUrl}
  statistics={stream}
  {isPlaying}
  {isLaunching}
  watchDisabled={!playbackReady || isPlaying || isLaunching}
  watchLabel={isLaunching
    ? `${stream.displayName} is launching`
    : isPlaying
      ? `${stream.displayName} is already playing`
      : `Watch ${stream.displayName}${desktop ? ' in MPV' : ''}`}
  onWatch={() => onWatch(stream)}
>
  {#snippet badges()}
    <LiveCardBadge tone="twitch">Live</LiveCardBadge>
  {/snippet}
  {#snippet fallback()}
    <span
      class="absolute inset-0 bg-[repeating-linear-gradient(135deg,transparent_0_18px,color-mix(in_srgb,var(--accent)_7%,transparent)_18px_19px)]"
    ></span>
    <Radio class="absolute bottom-5 left-5 size-10 text-white/15" />
  {/snippet}
  {#snippet shortcuts()}
    <CardShortcutButton
      platform="twitch"
      aria-label={`Copy ${stream.displayName} URL`}
      title="Copy Twitch URL"
      onclick={() => onCopy(stream)}
    >
      <Copy class="size-3.5" />
    </CardShortcutButton>
    <CardShortcutButton
      platform="twitch"
      aria-label={`Open ${stream.displayName} in browser`}
      title="Open in browser"
      onclick={() => onOpen(stream)}
    >
      <ExternalLink class="size-3.5" />
    </CardShortcutButton>
    <CardShortcutButton
      platform="twitch"
      aria-label={`Open ${stream.displayName} chat in browser`}
      title="Open chat in browser"
      onclick={() => onChat(stream)}
    >
      <MessageSquareText class="size-3.5" />
    </CardShortcutButton>
  {/snippet}
  {#snippet metadata()}
    <div class="mt-3 text-[0.62rem] tracking-[0.06em] text-muted uppercase">
      <span>{stream.gameName ?? 'Uncategorised'}</span>
    </div>
  {/snippet}
</LiveCard>
