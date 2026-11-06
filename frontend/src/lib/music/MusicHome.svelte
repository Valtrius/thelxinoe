<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { Music, Play } from '@lucide/svelte';
  import { api } from '../api';
  import { revisions } from '../server-events.svelte';
  import { navigationCache } from '../navigation-cache';
  import { inBackground } from '../page-readiness';
  import { activity, byMedia } from '../activity';
  import {
    attention,
    attentionErrors,
    acknowledgeAttention,
  } from '../attention';
  import { showToast } from '../providers/toasts';
  import type { MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaGrid from '../ui/MediaGrid.svelte';
  import LibraryCard from '../ui/LibraryCard.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import { emptyClass } from '../ui/styles';
  import TrackList from './TrackList.svelte';
  import TrackDetails from './TrackDetails.svelte';
  import {
    albumTracks,
    catalogQuery,
    listAll,
    plural,
    trackQueue,
    type MusicItem,
    type MusicPage,
  } from './library';
  import { kindOf, views, type ListSort, type View } from './list-view';

  let {
    view,
    sort,
    query,
    admin,
    counts = $bindable(),
    busy = $bindable(),
    play,
    openArtist,
    openAlbum,
  }: {
    view: View;
    sort: ListSort;
    query?: string;
    admin: boolean;
    counts: Partial<Record<string, number>>;
    busy: boolean;
    play: (choice: MediaChoice) => void;
    openArtist: (id: string) => void;
    openAlbum: (id: string, artist?: string | null) => void;
  } = $props();
  const kind = $derived(kindOf(view));
  onDestroy(() => {
    generation++;
  });

  const pageSize = 120;
  let items = $state<MusicItem[]>([]),
    listedKind = $state(''),
    total = $state(0),
    extending = $state(false),
    error = $state(''),
    pageError = $state(''),
    listed = '',
    generation = 0;
  // A list of another kind waits off screen until this one arrives.
  const shown = $derived(listedKind === kind ? items : []);
  const path = $derived(
    catalogQuery({
      kind,
      q: query || undefined,
      sort: sort.sort,
      order: sort.order,
    }),
  );
  async function load() {
    const request = ++generation;
    const current = path,
      listing = kind,
      q = query || undefined;
    busy = true;
    // A page still loading for the previous list is dropped with it.
    extending = false;
    error = '';
    pageError = '';
    try {
      await Promise.all([
        navigationCache.read(
          'catalog',
          current,
          () =>
            api<MusicPage>(
              `${current}&limit=${Math.max(pageSize, listed === current ? items.length : 0)}`,
            ),
          (page) => {
            if (request !== generation) return;
            listed = current;
            listedKind = listing;
            items = page.items;
            total = page.total ?? page.items.length;
          },
        ),
        ...views.map(([, , other]) =>
          navigationCache.read(
            'catalog',
            catalogQuery({ kind: other, q, limit: 0 }),
            () => api<MusicPage>(catalogQuery({ kind: other, q, limit: 0 })),
            (page) => {
              if (request === generation)
                counts = { ...counts, [other]: page.total };
            },
          ),
        ),
      ]);
    } catch (e) {
      if (request === generation) error = String(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  $effect(() => {
    void path;
    untrack(() => void load());
  });
  let observedRevision: number | undefined;
  $effect(() => {
    const next = revisions.catalog;
    untrack(() => {
      if (observedRevision !== undefined && observedRevision !== next) {
        navigationCache.invalidate('catalog');
        void load();
      }
      observedRevision = next;
    });
  });
  async function more() {
    if (busy || extending || pageError || items.length >= total) return;
    const request = generation,
      current = listed;
    extending = true;
    try {
      const page = await inBackground(() =>
        api<MusicPage>(`${current}&offset=${items.length}&limit=${pageSize}`),
      );
      if (request !== generation) return;
      const known = new Set(items.map((item) => item.id));
      const added = page.items.filter((item) => !known.has(item.id));
      items = [...items, ...added];
      total = added.length ? page.total : items.length;
      navigationCache.update('catalog', current, { items, total });
    } catch (e) {
      if (request === generation) pageError = String(e);
    } finally {
      if (request === generation) extending = false;
    }
  }
  function nearEnd(node: HTMLElement) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void more();
      },
      { root: node.closest('[data-feed-scroll]'), rootMargin: '150% 0px' },
    );
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }
  // A track plays on through the whole list, not only the pages shown.
  async function playTrack(track: MusicItem, fileId?: string) {
    const rest =
      items.length < total ? await listAll(listed).catch(() => items) : items;
    const queue = trackQueue(rest.length ? rest : items, track);
    if (queue) play({ ...queue, fileId });
  }
  async function playAlbum(album: MusicItem) {
    try {
      const queue = trackQueue(await albumTracks(album.id));
      if (queue) play(queue);
    } catch (e) {
      showToast({
        key: `album:${album.id}`,
        tone: 'error',
        title: album.title,
        message: String(e),
      });
    }
  }
  let detailed = $state<MusicItem | null>(null);

  const activityOf = $derived(byMedia($activity));
  const attentionOf = $derived(byMedia($attention));
  function seen(item: MusicItem) {
    for (const entry of attentionOf(item.id).filter(
      (entry) => entry.target !== 'music' || item.kind === 'album',
    ))
      void acknowledgeAttention(entry);
  }
  const caption = (item: MusicItem) =>
    item.kind === 'artist'
      ? [
          plural(item.album_count ?? 0, 'album'),
          plural(item.track_count ?? 0, 'track'),
        ].join(' · ')
      : [item.artist, item.year].filter(Boolean).join(' · ');
</script>

{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
<div data-page-region="music">
  {#if shown.length}
    {#if view === 'tracks'}<TrackList
        tracks={shown}
        play={(track) => void playTrack(track)}
        details={(track) => (detailed = track)}
        openAlbum={(track) => openAlbum(track.parent_id!, track.artist_id)}
        openArtist={(track) => openArtist(track.artist_id!)}
      />{:else}<MediaGrid
        revision={shown.map((i) => i.id).join(',')}
        label={view === 'artists' ? 'Artists' : 'Albums'}
      >
        {#each shown as item (item.id)}<LibraryCard
            {item}
            caption={caption(item)}
            activity={activityOf(item.id)}
            attention={attentionOf(item.id)}
            attentionError={attentionOf(item.id)
              .map((entry) => $attentionErrors[entry.id])
              .filter(Boolean)
              .join('\n')}
            onSeen={() => seen(item)}
            open={() =>
              item.kind === 'artist'
                ? openArtist(item.id)
                : openAlbum(item.id, item.parent_id)}
            >{#snippet menu()}{#if item.kind === 'album'}<button
                  class="grid size-7.5 place-items-center border border-[#ffffff30] bg-[#080b10c9] p-0 text-white"
                  title={`Play ${item.title}`}
                  aria-label={`Play ${item.title}`}
                  onclick={() => void playAlbum(item)}
                  ><Play size={14} fill="currentColor" /></button
                >{/if}{/snippet}</LibraryCard
          >{/each}
      </MediaGrid>{/if}
    {#if shown.length < total}
      {#if !busy && !extending && !pageError}<div use:nearEnd></div>{/if}
      {#if pageError}<Notice variant="error" role="alert"
          >{pageError}<Button
            variant="ghost"
            size="sm"
            onclick={() => {
              pageError = '';
              void more();
            }}>Try again</Button
          ></Notice
        >{:else if extending}{#if view === 'tracks'}<ContentSkeleton
            label="Loading more tracks"
            class="mt-3"
          />{:else}<MediaSkeleton
            label={`Loading more ${view}`}
            shape="square"
          />{/if}{/if}
    {/if}
  {:else if busy}{#if view === 'tracks'}<ContentSkeleton
        label="Loading tracks"
      />{:else}<MediaSkeleton
        label={`Loading ${view}`}
        shape="square"
      />{/if}{:else if !error}<section class={emptyClass}>
      <Music size={42} />
      <h2>{query ? 'No matches' : 'No music yet'}</h2>
      {#if query}<p>Try a different search.</p>{:else}<p>
          Imported music will appear here.
        </p>{/if}
    </section>{/if}
</div>
{#if detailed}<TrackDetails
    track={detailed}
    {admin}
    play={(fileId) => void playTrack(detailed!, fileId)}
    close={() => (detailed = null)}
    changed={() => void load()}
  />{/if}
