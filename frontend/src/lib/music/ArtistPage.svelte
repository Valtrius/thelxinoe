<script lang="ts">
  import { untrack } from 'svelte';
  import { Play, Shuffle } from '@lucide/svelte';
  import { revisions } from '../server-events.svelte';
  import { navigationCache } from '../navigation-cache';
  import { activity, byMedia } from '../activity';
  import {
    attention,
    attentionErrors,
    acknowledgeAttention,
  } from '../attention';
  import { showToast } from '../providers/toasts';
  import type { CatalogItem } from '../title/catalog';
  import type { MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaGrid from '../ui/MediaGrid.svelte';
  import LibraryCard from '../ui/LibraryCard.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import BackButton from './BackButton.svelte';
  import MusicArt from './MusicArt.svelte';
  import FavoriteToggle from './FavoriteToggle.svelte';
  import ManageButton from './ManageButton.svelte';
  import {
    albumTracks,
    artistTracks,
    artwork,
    catalogQuery,
    length,
    listAll,
    plural,
    trackQueue,
    type MusicItem,
  } from './library';

  let { artist, admin, play, openAlbum, backLabel, back } = $props<{
    artist: CatalogItem;
    admin: boolean;
    play: (choice: MediaChoice) => void;
    openAlbum: (id: string) => void;
    backLabel?: string;
    back: () => void;
  }>();
  let albums = $state<MusicItem[] | null>(null),
    error = $state(''),
    newest = $state(true),
    starting = $state<'play' | 'shuffle' | null>(null);
  const path = $derived(
    catalogQuery({
      parent: artist.id,
      sort: 'year',
      order: newest ? 'desc' : 'asc',
    }),
  );
  function load() {
    const current = path;
    error = '';
    navigationCache
      .read(
        'catalog',
        current,
        () => listAll(current),
        (items) => {
          if (path === current) albums = items;
        },
      )
      .catch((e) => {
        if (path === current) error = String(e);
      });
  }
  $effect(() => {
    void path;
    untrack(load);
  });
  let observedRevision: number | undefined;
  $effect(() => {
    const next = revisions.catalog;
    untrack(() => {
      if (observedRevision !== undefined && observedRevision !== next) {
        navigationCache.invalidate('catalog');
        load();
      }
      observedRevision = next;
    });
  });
  const tracks = $derived(
    albums?.reduce((sum, album) => sum + (album.track_count ?? 0), 0) ?? 0,
  );
  const duration = $derived(
    albums?.reduce((sum, album) => sum + (album.duration ?? 0), 0) ?? 0,
  );
  const art = $derived(
    artist.artwork_url ||
      albums?.find((album) => album.artwork_url)?.artwork_url ||
      '',
  );
  const facts = $derived(
    [
      'Artist',
      albums ? plural(albums.length, 'album') : '',
      albums ? plural(tracks, 'track') : '',
      length(duration),
    ]
      .filter(Boolean)
      .join(' · '),
  );
  async function start(shuffle: boolean) {
    starting = shuffle ? 'shuffle' : 'play';
    try {
      const queue = trackQueue(await artistTracks(artist.id));
      if (queue) play({ ...queue, shuffle });
    } catch (e) {
      showToast({
        key: `artist:${artist.id}`,
        tone: 'error',
        title: artist.title,
        message: String(e),
      });
    } finally {
      starting = null;
    }
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
  const activityOf = $derived(byMedia($activity));
  const attentionOf = $derived(byMedia($attention));
</script>

<BackButton label={backLabel} {back} />
<article class="grid min-w-0 gap-7" aria-label={artist.title}>
  <div
    class="relative isolate min-h-65 overflow-hidden border border-line bg-surface"
  >
    {#if art}<img
        src={artwork(art)}
        alt=""
        aria-hidden="true"
        class="absolute inset-0 -z-2 size-full scale-125 object-cover opacity-55 blur-3xl saturate-125"
      />{/if}
    <div
      class="absolute inset-0 -z-1 bg-linear-to-t from-background via-background/65 to-background/10"
    ></div>
    <div
      class="flex min-h-65 items-end gap-6 p-6.5 compact:flex-col compact:items-start compact:gap-4 compact:p-5"
    >
      <MusicArt
        url={art}
        class="size-37.5 border border-white/15 shadow-xl compact:size-28"
        sizes="10rem"
      />
      <div class="min-w-0">
        <p
          class="mb-0 text-[11px] font-semibold tracking-widest text-accent uppercase"
        >
          {facts}
        </p>
        <h1
          class="mt-2 text-4xl leading-tight font-semibold tracking-tight compact:text-3xl"
        >
          {artist.title}
        </h1>
        {#if artist.overview}<p
            class="mt-2.5 mb-0 line-clamp-2 max-w-[62ch] text-sm text-muted"
          >
            {artist.overview}
          </p>{/if}
        <div class="mt-4 flex flex-wrap items-center gap-2">
          <Button
            size="form"
            disabled={!tracks}
            loading={starting === 'play'}
            onclick={() => void start(false)}
            ><Play size={15} fill="currentColor" />Play all</Button
          >
          <Button
            variant="secondary"
            size="form"
            disabled={!tracks}
            loading={starting === 'shuffle'}
            onclick={() => void start(true)}
            ><Shuffle size={15} />Shuffle</Button
          >
          <FavoriteToggle id={artist.id} />
          {#if admin}<ManageButton
              id={artist.id}
              kind="artist"
              title={artist.title}
              changed={load}
            />{/if}
        </div>
      </div>
    </div>
  </div>
  <section class="grid min-w-0 gap-3" aria-label="Albums">
    <div class="flex min-h-8 flex-wrap items-center justify-between gap-3">
      <h2 class="mb-0 flex items-baseline gap-2 text-base">
        Albums{#if albums}<span class="text-xs font-normal text-muted"
            >{albums.length}</span
          >{/if}
      </h2>
      <div class="flex gap-1.5" role="group" aria-label="Album order">
        {#each [true, false] as order (order)}<button
            type="button"
            aria-pressed={newest === order}
            class={[
              'h-7 border px-2.5 text-xs',
              newest === order
                ? 'border-line-strong bg-accent-soft text-accent'
                : 'border-line text-muted hover:text-foreground',
            ]}
            onclick={() => (newest = order)}
            >{order ? 'Newest' : 'Oldest'}</button
          >{/each}
      </div>
    </div>
    {#if error}<Notice variant="error" role="alert">{error}</Notice
      >{:else if !albums}<MediaSkeleton
        label="Loading albums"
        shape="square"
        count={6}
      />{:else}<MediaGrid
        revision={albums.map((album) => album.id).join(',')}
        label="Albums"
      >
        {#each albums as album (album.id)}<LibraryCard
            item={album}
            keyPrefix="artist"
            caption={[album.year, plural(album.track_count ?? 0, 'track')]
              .filter(Boolean)
              .join(' · ')}
            activity={activityOf(album.id)}
            attention={attentionOf(album.id)}
            attentionError={attentionOf(album.id)
              .map((entry) => $attentionErrors[entry.id])
              .filter(Boolean)
              .join('\n')}
            onSeen={() => {
              for (const entry of attentionOf(album.id))
                void acknowledgeAttention(entry);
            }}
            open={() => openAlbum(album.id)}
            >{#snippet menu()}<button
                class="grid size-7.5 place-items-center border border-[#ffffff30] bg-[#080b10c9] p-0 text-white"
                title={`Play ${album.title}`}
                aria-label={`Play ${album.title}`}
                onclick={() => void playAlbum(album)}
                ><Play size={14} fill="currentColor" /></button
              >{/snippet}</LibraryCard
          >{/each}
      </MediaGrid>{/if}
  </section>
</article>
