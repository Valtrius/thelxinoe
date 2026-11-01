<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { ArrowRight, Disc3 } from '@lucide/svelte';
  import { api, type User } from '../api';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import MediaRow, { rowCardSizes } from '../seerr/MediaRow.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import type { Album, MusicService, RequestState } from './types';
  let {
    user,
    query,
    searching = $bindable(false),
    settings,
  } = $props<{
    user: User;
    query: string;
    searching?: boolean;
    settings: () => void;
  }>();
  type Feed = {
    id: string;
    label: string;
    items: Album[];
    loading: boolean;
    error: string;
  };
  const feed = (id: string, label: string): Feed => ({
    id,
    label,
    items: [],
    loading: true,
    error: '',
  });
  let service = $state<MusicService | null>(null),
    loading = $state(true),
    error = $state(''),
    feeds = $state<Feed[]>([
      feed('artists', 'New from your artists'),
      feed('trending', 'Trending albums'),
      feed('fresh', 'Fresh releases'),
    ]),
    results = $state<Album[]>([]),
    busy = $state<string | null>(null);
  let searchTimer: ReturnType<typeof setTimeout>,
    generation = 0,
    active = true;
  const term = $derived(query.trim());

  async function load() {
    loading = true;
    error = '';
    try {
      const { items } = await api<{ items: MusicService[] }>(
        '/acquisition/services',
      );
      if (!active) return;
      const lidarr = items.filter((item) => item.kind === 'lidarr');
      service = lidarr.find((item) => item.ready) ?? lidarr[0] ?? null;
    } catch (caught) {
      if (active) error = String(caught);
    } finally {
      if (active) loading = false;
    }
    if (service?.ready) for (const feed of feeds) void loadFeed(feed);
  }
  async function loadFeed(feed: Feed) {
    if (!service) return;
    feed.loading = true;
    feed.error = '';
    try {
      const { items } = await api<{ items: Album[] }>(
        `/acquisition/music/${feed.id}?service_id=${encodeURIComponent(service.id)}`,
      );
      if (active) feed.items = items;
    } catch (caught) {
      if (active) feed.error = String(caught);
    } finally {
      if (active) feed.loading = false;
    }
  }
  async function search() {
    const version = ++generation;
    if (!service?.ready || term.length < 2) {
      results = [];
      searching = false;
      return;
    }
    searching = true;
    error = '';
    try {
      const { items } = await api<{ items: Album[] }>(
        `/acquisition/search?service_id=${encodeURIComponent(service.id)}&term=${encodeURIComponent(term)}`,
      );
      if (active && version === generation) results = items;
    } catch (caught) {
      if (active && version === generation) error = String(caught);
    } finally {
      if (active && version === generation) searching = false;
    }
  }
  $effect(() => {
    void term;
    void service;
    clearTimeout(searchTimer);
    generation++;
    results = [];
    searching = !!service?.ready && term.length >= 2;
    searchTimer = setTimeout(() => void search(), 300);
  });
  async function request(album: Album) {
    if (!service) return;
    busy = album.external_id;
    error = '';
    try {
      const receipt = await api<{ id: string; state: RequestState }>(
        '/acquisition/requests',
        'POST',
        { service_id: service.id, external_id: album.external_id },
      );
      // The album can appear in several rows and in the results.
      for (const list of [...feeds.map((f) => f.items), results])
        for (const item of list)
          if (item.external_id === album.external_id)
            item.request = { id: receipt.id, state: receipt.state };
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = null;
    }
  }
  onMount(() => void load());
  onDestroy(() => {
    active = false;
    generation++;
    clearTimeout(searchTimer);
  });
</script>

{#snippet card(album: Album, sizes: string)}<AlbumCard
    {album}
    {request}
    {sizes}
    busy={busy === album.external_id}
  />{/snippet}

{#if error}<Notice variant="error" role="alert"
    >{error}<Button
      variant="ghost"
      size="sm"
      onclick={() => void (service ? search() : load())}>Try again</Button
    ></Notice
  >{/if}
{#if loading}<MediaSkeleton
    label="Loading music"
    heading={feeds[0].label}
    layout="poster-row"
    shape="square"
    count={6}
  />{:else if !service || !service.ready}<section
    class="grid min-h-85 place-content-center justify-items-center gap-4 text-center"
  >
    <Disc3 size={35} class="text-muted" />
    <h2 class="text-lg font-semibold">
      {service
        ? 'Lidarr needs acquisition defaults'
        : 'Connect Lidarr to request music'}
    </h2>
    <p class="max-w-110 text-sm leading-6 text-muted">
      {user.role !== 'admin'
        ? 'Your administrator needs to finish setting up Lidarr.'
        : service
          ? 'Choose its root folder and profiles in Media services.'
          : 'Install Lidarr in Media services to get started.'}
    </p>
    {#if user.role === 'admin'}<Button size="form" onclick={settings}
        >Open Media services <ArrowRight size={15} /></Button
      >{/if}
  </section>
{:else if term}
  {#if term.length < 2}<p class="py-8 text-sm text-muted">
      Enter at least two characters.
    </p>{:else if searching && !results.length}<MediaSkeleton
      label="Loading search results"
      layout="posters"
      shape="square"
    />{:else if !results.length}<p class="py-8 text-sm text-muted">
      No results for “{term}”.
    </p>{:else}<p class="mb-5 text-xs text-muted">Results for “{term}”</p>
    <div
      class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-3 compact:gap-3"
    >
      {#each results as album (album.external_id)}{@render card(
          album,
          '(max-width: 720px) 34vw, 19rem',
        )}{/each}
    </div>{/if}
{:else}
  <div class="grid gap-7">
    {#each feeds as feed (feed.id)}{#if feed.items.length}<MediaRow
          label={feed.label}
          >{#each feed.items as album (album.external_id)}{@render card(
              album,
              rowCardSizes,
            )}{/each}</MediaRow
        >{:else if feed.loading}<MediaSkeleton
          label={`Loading ${feed.label}`}
          heading={feed.label}
          layout="poster-row"
          shape="square"
          count={6}
        />{:else if feed.error}<Notice variant="error"
          ><strong>{feed.label}</strong>
          <p>{feed.error}</p>
          <Button variant="ghost" size="sm" onclick={() => void loadFeed(feed)}
            >Try again</Button
          ></Notice
        >{/if}{/each}
  </div>
{/if}
