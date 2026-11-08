<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    ArrowRight,
    ChevronDown,
    ChevronRight,
    Disc3,
    LibraryBig,
    Plus,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { plural } from '../music/library';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import MediaRow, { rowCardSizes } from '../seerr/MediaRow.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import AlbumCover from './AlbumCover.svelte';
  import AlbumSheet from './AlbumSheet.svelte';
  import ArtistPhoto from './ArtistPhoto.svelte';
  import MonitorButton from './MonitorButton.svelte';
  import {
    canRequest,
    lidarr,
    markRequested,
    requestable,
    requestLabel,
    requestMusic,
    scoped,
    type Album,
    type Artist,
    type MusicSearch,
    type MusicService,
  } from './types';
  let {
    user,
    query,
    searching = $bindable(false),
    settings,
    openArtist,
  } = $props<{
    user: User;
    query: string;
    searching?: boolean;
    settings: () => void;
    openArtist: (id: string) => void;
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
      feed('missing', 'More from your artists'),
      feed('similar', 'Similar artists'),
      feed('trending', 'Trending this week'),
    ]),
    result = $state<MusicSearch | null>(null),
    excludedOpen = $state(false),
    busy = $state<string | null>(null),
    sheet = $state<Album | null>(null);
  let searchTimer: ReturnType<typeof setTimeout>,
    generation = 0,
    active = true;
  const term = $derived(query.trim());
  const lists = () => [
    ...feeds.map((feed) => feed.items),
    ...(result ? [result.albums, result.matches, result.excluded] : []),
  ];

  async function load() {
    loading = true;
    error = '';
    try {
      const found = await lidarr();
      if (active) service = found;
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
      const { items, artist } = await api<{
        items: Album[];
        artist?: { name: string } | null;
      }>(scoped(`/acquisition/music/feeds/${feed.id}`, service));
      if (!active) return;
      feed.items = items;
      if (artist) feed.label = `Similar to ${artist.name}`;
    } catch (caught) {
      if (active) feed.error = String(caught);
    } finally {
      if (active) feed.loading = false;
    }
  }
  async function search() {
    const version = ++generation;
    if (!service?.ready || term.length < 2) {
      result = null;
      searching = false;
      return;
    }
    searching = true;
    error = '';
    try {
      const found = await api<MusicSearch>(
        scoped(
          `/acquisition/music/search?term=${encodeURIComponent(term)}`,
          service,
        ),
      );
      if (active && version === generation) {
        result = found;
        excludedOpen = false;
      }
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
    result = null;
    searching = !!service?.ready && term.length >= 2;
    searchTimer = setTimeout(() => void search(), 300);
  });
  async function request(album: Album) {
    if (!service) return;
    busy = album.external_id;
    error = '';
    try {
      const receipt = await requestMusic(service, album.external_id);
      // The album can appear in several rows and in the results.
      markRequested(lists(), album.external_id, receipt);
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
    {openArtist}
    open={(album) => (sheet = album)}
    busy={busy === album.external_id}
  />{/snippet}

{#snippet chip(text: string)}<span
    class="inline-flex h-5.5 shrink-0 items-center border border-line px-2 text-[10px] text-muted"
    >{text}</span
  >{/snippet}

{#snippet row(album: Album)}<div
    class="flex items-center gap-3 border-t border-line px-3 py-2.5 first:border-t-0"
  >
    <button
      type="button"
      class="flex min-w-0 flex-1 items-center gap-3 text-left"
      onclick={() => (sheet = album)}
      ><AlbumCover
        src={album.cover}
        title={album.title}
        artist={album.artist}
        small
        sizes="2.5rem"
        class="size-10 shrink-0 border border-line"
      /><span class="min-w-0"
        ><span class="block truncate text-[13px] font-semibold"
          >{album.title}</span
        ><span class="block truncate text-[11px] text-muted"
          >{[album.artist, album.year].filter(Boolean).join(' · ')}</span
        ></span
      ></button
    >{@render chip(album.type ?? 'Album')}
    {#if album.in_library}<span
        class="inline-flex items-center gap-1 text-[11px] text-accent"
        ><LibraryBig size={12} />In library</span
      >{:else if album.request && !requestable(album.request.state)}<span
        class="text-[11px] text-muted">{requestLabel(album.request.state)}</span
      >{:else if album.missing}<span class="text-[11px] text-warning"
        >Missing</span
      >{:else if canRequest(album)}<Button
        variant="secondary"
        size="sm"
        loading={busy === album.external_id}
        aria-label={`Request ${album.title}`}
        onclick={() => void request(album)}><Plus size={14} />Request</Button
      >{/if}
  </div>{/snippet}

{#snippet artistLink(artist: Artist)}<button
    type="button"
    class="flex w-full items-center gap-3 border-t border-line px-3 py-2 text-left first:border-t-0 hover:bg-surface-soft"
    onclick={() => openArtist(artist.id)}
    ><ArtistPhoto src={artist.image} class="size-7.5" /><span class="min-w-0"
      ><span class="block truncate text-xs font-semibold">{artist.name}</span
      ><span class="block truncate text-[10px] text-muted"
        >{[artist.type, artist.disambiguation]
          .filter(Boolean)
          .join(' · ')}</span
      ></span
    ></button
  >{/snippet}

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
    </p>{:else if !result}{#if searching}<MediaSkeleton
        label="Loading search results"
        layout="posters"
        shape="square"
      />{/if}{:else if !result.artist && !result.artists.length && !result.matches.length && !result.excluded.length}<p
      class="py-8 text-sm text-muted"
    >
      No results for “{term}”.
    </p>{:else}<div class="grid gap-7">
      {#if result.artist}{@const top = result.artist}
        <section class="flex flex-wrap gap-4" aria-label="Artists">
          <article
            class="flex min-w-0 flex-[999_1_24rem] items-center gap-4.5 border border-line bg-surface p-4 compact:items-start"
            aria-label={top.name}
          >
            <ArtistPhoto src={top.image} class="size-26 compact:size-20" />
            <div class="grid min-w-0 flex-1 gap-2">
              <p
                class="text-[10px] font-semibold tracking-[0.14em] text-accent uppercase"
              >
                Artist · {plural(top.album_count ?? 0, 'album')}
              </p>
              <h2
                class="text-2xl leading-tight font-semibold tracking-tight compact:text-xl"
              >
                {top.name}
              </h2>
              {#if top.type || top.genres.length}<div
                  class="flex flex-wrap gap-1.5"
                >
                  {#each [top.type, ...top.genres.slice(0, 2)].filter(Boolean) as text (text)}{@render chip(
                      text!,
                    )}{/each}
                </div>{/if}
              <div class="mt-1 flex flex-wrap gap-2">
                <Button size="form" onclick={() => openArtist(top.id)}
                  >Open artist<ArrowRight size={15} /></Button
                >
                <MonitorButton artist={top} {service} {user} />
              </div>
            </div>
          </article>
          {#if result.artists.length}<div class="min-w-0 flex-[1_1_12rem]">
              <h3 class="mb-1.5 text-xs font-semibold text-muted">
                Other artists
              </h3>
              <div class="border border-line bg-surface">
                {#each result.artists as artist (artist.id)}{@render artistLink(
                    artist,
                  )}{/each}
              </div>
            </div>{/if}
        </section>
        {#if result.albums.length}<section
            class="grid gap-4"
            aria-label={`Albums by ${top.name}`}
          >
            <h2 class="flex items-baseline gap-2 text-base font-semibold">
              Albums by {top.name}<span class="text-xs font-normal text-muted"
                >{result.albums.length}</span
              >
            </h2>
            <div
              class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-3 compact:gap-3"
            >
              {#each result.albums as album (album.external_id)}{@render card(
                  album,
                  '(max-width: 720px) 34vw, 12rem',
                )}{/each}
            </div>
          </section>{/if}
      {:else if result.artists.length}<section
          class="grid gap-3"
          aria-label="Artists"
        >
          <h2 class="text-base font-semibold">Artists</h2>
          <div class="border border-line bg-surface">
            {#each result.artists as artist (artist.id)}{@render artistLink(
                artist,
              )}{/each}
          </div>
        </section>{/if}
      {#if result.matches.length || result.excluded.length}{@const label =
          result.artist ? 'Other matches' : 'Albums'}
        <section class="grid gap-3" aria-label={label}>
          <h2 class="text-base font-semibold">{label}</h2>
          <div class="border border-line bg-surface">
            {#each result.matches as album (album.external_id)}{@render row(
                album,
              )}{/each}
            {#if result.excluded.length}<button
                type="button"
                class="flex w-full items-center gap-3 border-t border-line px-3 py-2.5 text-left text-xs font-semibold text-muted first:border-t-0 hover:text-foreground"
                aria-expanded={excludedOpen}
                onclick={() => (excludedOpen = !excludedOpen)}
                >{#if excludedOpen}<ChevronDown size={16} />{:else}<ChevronRight
                    size={16}
                  />{/if}<span class="flex-1"
                  >{plural(result.excluded.length, 'other release')}</span
                >{@render chip('Not requestable')}</button
              >{#if excludedOpen}{#each result.excluded as album (album.external_id)}{@render row(
                    album,
                  )}{/each}{/if}{/if}
          </div>
        </section>{/if}
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

{#if sheet && service}<AlbumSheet
    album={sheet}
    {service}
    close={() => (sheet = null)}
    openArtist={(id) => {
      sheet = null;
      openArtist(id);
    }}
    requested={(album, request) =>
      markRequested(lists(), album.external_id, request)}
  />{/if}
