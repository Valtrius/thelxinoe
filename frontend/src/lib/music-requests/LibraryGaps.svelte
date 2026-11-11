<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { ArrowRight } from '@lucide/svelte';
  import { api } from '../api';
  import { inBackground } from '../page-readiness';
  import { showToast } from '../providers/toasts';
  import Button from '../ui/Button.svelte';
  import MediaRow, { rowCardSizes } from '../seerr/MediaRow.svelte';
  import AlbumCard from './AlbumCard.svelte';
  import AlbumSheet from './AlbumSheet.svelte';
  import {
    lidarr,
    markRequested,
    requestMusic,
    scoped,
    type Album,
    type ArtistView,
    type MusicService,
  } from './types';
  let {
    id,
    discography,
  }: {
    /** The artist's MusicBrainz ID. */
    id: string;
    discography: () => void;
  } = $props();
  let service = $state<MusicService | null>(null),
    albums = $state<Album[]>([]),
    busy = $state<string | null>(null),
    sheet = $state<Album | null>(null);
  let active = true;

  // The library page neither waits for it nor needs it: a missing Lidarr or MusicBrainz
  // only hides the row.
  async function load() {
    try {
      const found = await inBackground(lidarr);
      if (!active || !found?.ready) return;
      const view = await inBackground(() =>
        api<ArtistView>(
          scoped(`/acquisition/music/artists/${encodeURIComponent(id)}`, found),
        ),
      );
      if (!active) return;
      service = found;
      albums = view.albums.filter((album) => !album.in_library);
    } catch {
      // Nothing to add to the page.
    }
  }
  async function request(album: Album) {
    if (!service) return;
    busy = album.external_id;
    try {
      markRequested(
        [albums],
        album.external_id,
        await requestMusic(service, album.external_id),
      );
    } catch (caught) {
      showToast({
        key: `request:${album.external_id}`,
        tone: 'error',
        title: album.title,
        message: String(caught),
      });
    } finally {
      busy = null;
    }
  }
  onMount(() => void load());
  onDestroy(() => (active = false));
</script>

{#if service && albums.length}<MediaRow label="Not in your library"
    >{#snippet action()}<Button variant="ghost" size="sm" onclick={discography}
        >Full discography<ArrowRight size={14} /></Button
      >{/snippet}{#each albums as album (album.external_id)}<AlbumCard
        {album}
        {request}
        sizes={rowCardSizes}
        byArtist={false}
        busy={busy === album.external_id}
        open={(album) => (sheet = album)}
      />{/each}</MediaRow
  >{/if}

{#if sheet && service}<AlbumSheet
    album={sheet}
    {service}
    close={() => (sheet = null)}
    requested={(album, request) =>
      markRequested([albums], album.external_id, request)}
  />{/if}
