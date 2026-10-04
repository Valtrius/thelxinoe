<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { RefreshCw, Radio } from '@lucide/svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { api, type User } from './api';
  import type { MediaChoice } from './playback';
  import { providers } from './providers/availability';
  import {
    api as providerApi,
    type OnlineAccount,
    type KickFeed,
  } from './providers/api';
  import type { YoutubeVideo, TwitchLiveStream } from './providers/types';
  import { LatestRequest } from './latest-request';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import LibraryCard from './ui/LibraryCard.svelte';
  import MediaRow from './ui/MediaRow.svelte';
  import MediaSkeleton from './ui/MediaSkeleton.svelte';
  import LiveCard from './providers/components/ui/LiveCard.svelte';
  import LiveCardBadge from './providers/components/ui/LiveCardBadge.svelte';
  import PlatformIcon from './ui/PlatformIcon.svelte';

  let { user, revision, accountRevision, playing, play, navigate, details } =
    $props<{
      user: User;
      revision: number;
      accountRevision: number;
      playing: MediaChoice | null;
      play: (choice: MediaChoice) => Promise<void>;
      navigate: (section: string) => void;
      details: (id: string, kind: string) => void;
    }>();

  type Item = {
    id: string;
    kind: string;
    title: string;
    available: boolean;
    artwork_url?: string;
    year?: number | null;
    show_title?: string;
    position?: number;
    duration?: number;
    fileId?: string;
    last_played?: number;
  };
  type Library = {
    continue_watching: Item[];
    next_up: Item[];
    watch_later: Item[];
    favorites: Item[];
  };
  type YoutubeHome = {
    continue_watching: (YoutubeVideo & { last_played: number })[];
    next_up: YoutubeVideo[];
  };
  type Stream = {
    platform: 'twitch' | 'kick';
    id: string;
    name: string;
    title: string;
    thumbnail?: string | null;
    avatar?: string | null;
    category?: string | null;
    viewerCount: number;
    startedAt: string;
  };
  const emptyLibrary = (): Library => ({
    continue_watching: [],
    next_up: [],
    watch_later: [],
    favorites: [],
  });
  let library = $state<Library>(emptyLibrary());
  let youtube = $state<YoutubeHome>({ continue_watching: [], next_up: [] });
  let twitch = $state<TwitchLiveStream[]>([]);
  let kick = $state<KickFeed['items']>([]);
  let errors = $state({ library: '', youtube: '', twitch: '', kick: '' });
  let loading = $state({
    library: true,
    youtube: true,
    twitch: true,
    kick: true,
  });
  let progressRefresh = $state(0);
  let loaded = $state({
    library: false,
    youtube: false,
    twitch: false,
    kick: false,
  });
  const initialLoading = $derived({
    library: loading.library && !loaded.library,
    youtube: loading.youtube && !loaded.youtube,
    twitch: loading.twitch && !loaded.twitch,
    kick: loading.kick && !loaded.kick,
  });
  let progressReady = false;
  let libraryAgain = false;
  let youtubeAgain = false;
  let owner = '';
  const requests = {
    library: new LatestRequest(),
    youtube: new LatestRequest(),
    twitch: new LatestRequest(),
    kick: new LatestRequest(),
  };
  const pending = new SvelteSet<string>();
  const busy = $derived(Object.values(loading).some(Boolean));
  function youtubeItem(video: YoutubeVideo & { last_played?: number }): Item {
    return {
      id: `youtube:${video.videoId}`,
      kind: 'youtube',
      title: video.title,
      available: true,
      artwork_url: video.thumbnailUrl ?? undefined,
      show_title: video.channelName,
      position: video.positionSeconds,
      duration: video.durationSeconds ?? undefined,
      last_played: video.last_played,
    };
  }
  const continuing = $derived(
    [
      ...library.continue_watching,
      ...youtube.continue_watching.map(youtubeItem),
    ].sort(
      (a, b) =>
        (b.last_played ?? 0) - (a.last_played ?? 0) || a.id.localeCompare(b.id),
    ),
  );
  const streams = $derived<Stream[]>(
    [
      ...twitch.map((stream) => ({
        platform: 'twitch' as const,
        id: `twitch:${stream.broadcasterId}`,
        name: stream.displayName,
        title: stream.title,
        thumbnail: stream.thumbnailUrl
          ?.replace('{width}', '640')
          .replace('{height}', '360'),
        avatar: stream.profileImageUrl,
        category: stream.gameName,
        viewerCount: stream.viewerCount,
        startedAt: stream.startedAt,
      })),
      ...kick.map((stream) => ({
        platform: 'kick' as const,
        id: `kick:${stream.slug}`,
        name: stream.display_name || stream.slug,
        title: stream.title,
        thumbnail: stream.thumbnail_url,
        avatar: stream.profile_image_url,
        category: stream.category,
        viewerCount: stream.viewers,
        startedAt: stream.started_at ?? '',
      })),
    ].sort((a, b) => b.viewerCount - a.viewerCount || a.id.localeCompare(b.id)),
  );
  const hasContent = $derived(
    continuing.length ||
      library.next_up.length ||
      youtube.next_up.length ||
      streams.length ||
      library.watch_later.length ||
      library.favorites.length,
  );

  async function loadLibrary() {
    const current = requests.library.begin();
    loading.library = true;
    errors.library = '';
    try {
      const result = await api<Library>('/me/home');
      if (current()) library = result;
    } catch (error) {
      if (current()) errors.library = String(error);
    } finally {
      if (current()) {
        loading.library = false;
        loaded.library = true;
        if (libraryAgain) {
          libraryAgain = false;
          void loadLibrary();
        }
      }
    }
  }
  async function loadYoutube(enabled: boolean) {
    const current = requests.youtube.begin();
    errors.youtube = '';
    loading.youtube = enabled;
    if (!enabled) {
      youtube = { continue_watching: [], next_up: [] };
      loaded.youtube = true;
      return;
    }
    try {
      const account = await api<OnlineAccount>('/online/youtube');
      if (!current()) return;
      if (account.account.status !== 'connected') {
        youtube = { continue_watching: [], next_up: [] };
        if (account.account.status === 'reconnect_required')
          errors.youtube = 'Reconnect YouTube';
        return;
      }
      const result = await api<YoutubeHome>('/online/youtube/home');
      if (current()) {
        youtube = result;
        errors.youtube = account.sync?.error ?? '';
      }
    } catch (error) {
      if (current()) {
        youtube = { continue_watching: [], next_up: [] };
        errors.youtube = String(error);
      }
    } finally {
      if (current()) {
        loading.youtube = false;
        loaded.youtube = true;
        if (youtubeAgain) {
          youtubeAgain = false;
          void loadYoutube(enabled);
        }
      }
    }
  }
  async function loadTwitch(enabled: boolean) {
    const current = requests.twitch.begin();
    errors.twitch = '';
    loading.twitch = enabled;
    if (!enabled) {
      twitch = [];
      loaded.twitch = true;
      return;
    }
    try {
      const account = await api<OnlineAccount>('/online/twitch');
      if (!current()) return;
      if (account.account.status !== 'connected') {
        twitch = [];
        if (account.account.status === 'reconnect_required')
          errors.twitch = 'Reconnect Twitch';
        return;
      }
      if (
        account.sync?.error ||
        !account.sync?.last_complete ||
        account.sync.last_complete < Date.now() / 1000 - 300
      ) {
        twitch = [];
        errors.twitch =
          account.sync?.error || 'Twitch live status is out of date';
        return;
      }
      const result = await providerApi.twitchStreams();
      if (current()) twitch = result;
    } catch (error) {
      if (current()) {
        twitch = [];
        errors.twitch = String(error);
      }
    } finally {
      if (current()) {
        loading.twitch = false;
        loaded.twitch = true;
      }
    }
  }
  async function loadKick(enabled: boolean) {
    const current = requests.kick.begin();
    errors.kick = '';
    loading.kick = enabled;
    if (!enabled) {
      kick = [];
      loaded.kick = true;
      return;
    }
    try {
      const result = await api<KickFeed>('/online/kick');
      if (!current()) return;
      if (!result.connected || !result.configured) {
        kick = [];
        return;
      }
      const stale = result.items.filter(
        (item) =>
          item.error ||
          (item.live === true && item.updated_at < Date.now() / 1000 - 300),
      );
      kick = result.items.filter(
        (item) =>
          item.live === true &&
          !item.error &&
          item.updated_at >= Date.now() / 1000 - 300,
      );
      errors.kick =
        stale[0]?.error ||
        (stale.length ? 'Kick live status is out of date' : '');
    } catch (error) {
      if (current()) {
        kick = [];
        errors.kick = String(error);
      }
    } finally {
      if (current()) {
        loading.kick = false;
        loaded.kick = true;
      }
    }
  }
  $effect(() => {
    const id = user.id;
    const enabled = $providers;
    void accountRevision;
    untrack(() => {
      if (id !== owner) {
        owner = id;
        loaded = { library: false, youtube: false, twitch: false, kick: false };
        library = emptyLibrary();
        youtube = { continue_watching: [], next_up: [] };
        twitch = [];
        kick = [];
      }
      libraryAgain = false;
      youtubeAgain = false;
      void loadLibrary();
      void loadYoutube(enabled.youtube);
      void loadTwitch(enabled.twitch);
      void loadKick(enabled.kick);
    });
  });
  $effect(() => {
    void revision;
    void progressRefresh;
    untrack(() => {
      if (!progressReady) {
        progressReady = true;
        return;
      }
      if (loading.library) libraryAgain = true;
      else void loadLibrary();
      if (loading.youtube) youtubeAgain = true;
      else void loadYoutube($providers.youtube);
    });
  });
  function refreshSources() {
    if (loading.library) libraryAgain = true;
    else void loadLibrary();
    if (loading.youtube) youtubeAgain = true;
    else void loadYoutube($providers.youtube);
    if (!loading.twitch) void loadTwitch($providers.twitch);
    if (!loading.kick) void loadKick($providers.kick);
  }
  onMount(() => {
    const changed = () => progressRefresh++;
    const timer = setInterval(refreshSources, 60_000);
    window.addEventListener('thelxinoe-youtube-progress', changed);
    return () => {
      clearInterval(timer);
      window.removeEventListener('thelxinoe-youtube-progress', changed);
      Object.values(requests).forEach((request) => request.invalidate());
    };
  });
  async function launch(choice: MediaChoice) {
    if (pending.has(choice.id) || playing?.id === choice.id) return;
    pending.add(choice.id);
    try {
      await play(choice);
    } finally {
      pending.delete(choice.id);
    }
  }
  function playItem(item: Item, resume = false) {
    if (['show', 'season', 'artist', 'album'].includes(item.kind)) {
      details(item.id, item.kind);
      return;
    }
    void launch({
      id: item.id,
      kind: item.kind,
      title: item.title,
      fileId: item.fileId,
      position: resume
        ? item.position
        : item.kind === 'youtube'
          ? 0
          : undefined,
    });
  }
</script>

<div class="grid min-w-0 gap-7" aria-busy={busy}>
  <div class="flex justify-end">
    <Button
      variant="ghost"
      size="sm"
      aria-label="Refresh Home"
      onclick={refreshSources}
      disabled={busy}
      ><RefreshCw
        size={17}
        class={busy ? 'animate-spin motion-reduce:animate-none' : ''}
      /></Button
    >
  </div>
  {#each Object.entries(errors).filter(([, message]) => message) as [source, message] (source)}
    <Notice variant="callout" role="status">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <span
          >{source === 'library'
            ? 'Library'
            : source === 'youtube'
              ? 'YouTube'
              : source === 'twitch'
                ? 'Twitch'
                : 'Kick'}: {message}</span
        >
        <Button
          variant="ghost"
          size="sm"
          onclick={() =>
            source === 'library'
              ? void loadLibrary()
              : navigate(
                  source === 'youtube'
                    ? 'YouTube'
                    : source === 'twitch'
                      ? 'Twitch'
                      : 'Kick',
                )}
          >{source === 'library'
            ? 'Retry'
            : 'Open ' +
              (source === 'youtube'
                ? 'YouTube'
                : source === 'twitch'
                  ? 'Twitch'
                  : 'Kick')}</Button
        >
      </div>
    </Notice>
  {/each}
  {#if continuing.length}
    <MediaRow label="Continue watching">
      {#each continuing as item (item.id)}
        <LibraryCard
          {item}
          landscape
          keyPrefix="home-continue"
          open={() => playItem(item, true)}
          play={() => playItem(item, true)}
          details={item.kind === 'youtube'
            ? undefined
            : () => details(item.id, item.kind)}
        />
      {/each}
    </MediaRow>
  {:else if initialLoading.library || initialLoading.youtube}
    <MediaSkeleton
      label="Loading continue watching"
      heading="Continue watching"
      layout="row"
      shape="landscape"
    />
  {/if}
  {#if library.next_up.length || youtube.next_up.length || initialLoading.library || initialLoading.youtube}
    <div class="grid min-w-0 gap-6">
      {#if library.next_up.length}
        <MediaRow label="Next up">
          {#each library.next_up as item (item.id)}
            <LibraryCard
              {item}
              landscape
              keyPrefix="home-next"
              open={() => playItem(item)}
              play={() => playItem(item)}
              details={() => details(item.id, item.kind)}
            />
          {/each}
        </MediaRow>
      {:else if initialLoading.library}
        <MediaSkeleton
          label="Loading library"
          heading="Next up"
          layout="row"
          shape="landscape"
        />
      {/if}
      {#if youtube.next_up.length}
        <MediaRow label="YouTube subscriptions">
          {#each youtube.next_up.map(youtubeItem) as item (item.id)}
            <LibraryCard
              {item}
              landscape
              keyPrefix="home-youtube"
              open={() => playItem(item)}
              play={() => playItem(item)}
            />
          {/each}
        </MediaRow>
      {:else if initialLoading.youtube}
        <MediaSkeleton
          label="Loading YouTube subscriptions"
          heading="YouTube subscriptions"
          layout="row"
          shape="landscape"
        />
      {/if}
    </div>
  {/if}
  {#if streams.length}
    <MediaRow label="Live now">
      {#each streams as stream (stream.id)}
        <LiveCard
          platform={stream.platform}
          streamId={stream.id}
          displayName={stream.name}
          title={stream.title}
          thumbnailUrl={stream.thumbnail}
          profileImageUrl={stream.avatar}
          statistics={stream}
          isPlaying={playing?.id === stream.id}
          isLaunching={pending.has(stream.id)}
          watchDisabled={playing?.id === stream.id || pending.has(stream.id)}
          watchLabel={`Watch ${stream.name}`}
          onWatch={() => void launch({ id: stream.id, title: stream.title })}
        >
          {#snippet badges()}<LiveCardBadge tone={stream.platform}
              >{stream.platform === 'twitch' ? 'Twitch' : 'Kick'} · Live</LiveCardBadge
            >{/snippet}
          {#snippet fallback()}<Radio
              class="absolute inset-0 m-auto size-10 text-accent"
            />{/snippet}
          {#snippet shortcuts()}{/snippet}
          {#snippet metadata()}<div
              class="mt-3 flex items-center gap-2 text-xs text-muted"
            >
              <PlatformIcon
                platform={stream.platform}
                class="size-4 shrink-0"
              />{stream.category ?? ''}
            </div>{/snippet}
        </LiveCard>
      {/each}
    </MediaRow>
  {:else if initialLoading.twitch || initialLoading.kick}
    <MediaSkeleton
      label="Loading live streams"
      heading="Live now"
      layout="row"
      shape="landscape"
    />
  {/if}
  {#each [{ label: 'Watch later', items: library.watch_later }, { label: 'Favorites', items: library.favorites }] as shelf (shelf.label)}
    {#if shelf.items.length}
      <MediaRow label={shelf.label}>
        {#each shelf.items as item (item.id)}
          <LibraryCard
            {item}
            landscape
            keyPrefix={`home-${shelf.label}`}
            open={() => details(item.id, item.kind)}
            play={['movie', 'episode', 'track'].includes(item.kind)
              ? () => playItem(item)
              : undefined}
            details={() => details(item.id, item.kind)}
          />
        {/each}
      </MediaRow>
    {/if}
  {/each}
  {#if !busy && !hasContent}
    <nav aria-label="Browse media" class="flex flex-wrap gap-2">
      {#each ['Movies', 'Shows', 'Music', 'Discover'] as name (name)}<Button
          variant="ghost"
          onclick={() => navigate(name)}>{name}</Button
        >{/each}
    </nav>
  {/if}
</div>
