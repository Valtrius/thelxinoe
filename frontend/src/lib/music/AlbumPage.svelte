<script lang="ts">
  import { untrack } from 'svelte';
  import { Play, Shuffle } from '@lucide/svelte';
  import { revisions } from '../server-events.svelte';
  import { navigationCache } from '../navigation-cache';
  import type { CatalogItem } from '../title/catalog';
  import type { MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import BackButton from './BackButton.svelte';
  import MusicArt from './MusicArt.svelte';
  import FavoriteToggle from './FavoriteToggle.svelte';
  import ManageButton from './ManageButton.svelte';
  import AddToPlaylist from './AddToPlaylist.svelte';
  import TrackList from './TrackList.svelte';
  import TrackDetails from './TrackDetails.svelte';
  import {
    albumTracks,
    audioFormat,
    catalogQuery,
    length,
    plural,
    trackQueue,
    type MusicItem,
  } from './library';

  let { album, artistName, admin, play, openArtist, backLabel, back, focus } =
    $props<{
      album: CatalogItem;
      artistName: string;
      admin: boolean;
      play: (choice: MediaChoice) => void;
      openArtist?: () => void;
      backLabel?: string;
      back: () => void;
      /** A track the page was opened for. */
      focus?: string;
    }>();
  let tracks = $state<MusicItem[] | null>(null),
    error = $state(''),
    detailed = $state<MusicItem | null>(null);
  function load() {
    const id = album.id;
    error = '';
    navigationCache
      .read(
        'catalog',
        catalogQuery({ parent: id }),
        () => albumTracks(id),
        (items) => {
          if (album.id === id) tracks = items;
        },
      )
      .catch((e) => {
        if (album.id === id) error = String(e);
      });
  }
  $effect(() => {
    void album.id;
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
  // The page opened for a track scrolls to it once the list is shown.
  $effect(() => {
    if (!tracks || !focus) return;
    const id = focus;
    requestAnimationFrame(() =>
      document
        .querySelector(`[data-track="${CSS.escape(id)}"]`)
        ?.scrollIntoView({ block: 'center' }),
    );
  });
  const duration = $derived(
    tracks?.reduce((sum, track) => sum + (track.duration ?? 0), 0) ?? 0,
  );
  // One format for the album, when every track shares it.
  const format = $derived.by(() => {
    const formats = new Set(tracks?.map((track) => audioFormat(track.audio)));
    return formats.size === 1 ? [...formats][0] : '';
  });
  const facts = $derived(
    [
      tracks ? plural(tracks.length, 'track') : '',
      length(duration),
      format,
    ].filter(Boolean),
  );
  function start(from?: MusicItem, shuffle = false, fileId?: string) {
    const queue = tracks && trackQueue(tracks, from);
    if (queue) play({ ...queue, shuffle, fileId });
  }
</script>

<BackButton label={backLabel} {back} />
<article class="grid min-w-0 gap-6" aria-label={album.title}>
  <div
    class="flex items-end gap-6 compact:flex-col compact:items-start compact:gap-4"
  >
    <MusicArt
      data-sidebar-resize="xy-pos"
      url={album.artwork_url}
      class="size-50 border border-white/15 shadow-xl compact:size-36"
      sizes="13rem"
    />
    <div class="min-w-0 pb-0.5">
      <p
        class="mb-0 text-[11px] font-semibold tracking-widest text-accent uppercase"
      >
        Album{album.year ? ` · ${album.year}` : ''}
      </p>
      <h1
        class="mt-2 text-3xl leading-tight font-semibold tracking-tight compact:text-2xl"
      >
        {album.title}
      </h1>
      {#if artistName}{#if openArtist}<button
            type="button"
            class="mt-1.5 bg-transparent p-0 text-sm text-accent hover:underline"
            onclick={openArtist}>{artistName}</button
          >{:else}<p class="mt-1.5 mb-0 text-sm">{artistName}</p>{/if}{/if}
      {#if facts.length}<p
          class="mt-3 mb-0 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted"
        >
          {#each facts as fact (fact)}<span>{fact}</span>{/each}
        </p>{/if}
      {#if album.overview}<p
          class="mt-3 mb-0 line-clamp-3 max-w-[62ch] text-sm text-muted"
        >
          {album.overview}
        </p>{/if}
      <div class="mt-4 flex flex-wrap items-center gap-2">
        <Button
          size="form"
          disabled={!tracks?.some((track) => track.available)}
          onclick={() => start()}
          ><Play size={15} fill="currentColor" />Play</Button
        >
        <Button
          variant="secondary"
          size="form"
          disabled={!tracks?.some((track) => track.available)}
          onclick={() => start(undefined, true)}
          ><Shuffle size={15} />Shuffle</Button
        >
        <FavoriteToggle id={album.id} />
        <AddToPlaylist tracks={() => tracks ?? []} disabled={!tracks?.length} />
        {#if admin}<ManageButton
            id={album.id}
            kind="album"
            title={album.title}
            changed={load}
          />{/if}
      </div>
    </div>
  </div>
  {#if error}<Notice variant="error" role="alert">{error}</Notice
    >{:else if !tracks}<ContentSkeleton
      label="Loading tracks"
      count={8}
    />{:else}<TrackList
      {tracks}
      album
      play={(track) => start(track)}
      details={(track) => (detailed = track)}
      openArtist={openArtist ? () => openArtist() : undefined}
    />{/if}
</article>
{#if detailed}<TrackDetails
    track={detailed}
    {admin}
    play={(fileId) => start(detailed!, false, fileId)}
    close={() => (detailed = null)}
    changed={load}
  />{/if}
