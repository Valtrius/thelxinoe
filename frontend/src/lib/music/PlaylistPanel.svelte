<script lang="ts">
  import { untrack } from 'svelte';
  import type { AppRoute } from '../navigation';
  import type { MediaChoice } from '../playback';
  import { compactViewport } from '../responsive';
  import Drawer from '../ui/Drawer.svelte';
  import PlaylistSidebar from './PlaylistSidebar.svelte';
  import TrackDetails from './TrackDetails.svelte';
  import { playlistQueue, type PlaylistTrack } from './playlist-order';
  import { playlists } from './playlists.svelte';

  let {
    route,
    userId,
    admin,
    play,
    go,
    updateFilters,
  }: {
    route: AppRoute;
    userId: string;
    admin: boolean;
    play: (choice: MediaChoice) => void;
    go: (route: AppRoute) => void;
    updateFilters: (route: AppRoute) => void;
  } = $props();

  let detailed = $state<PlaylistTrack | null>(null);
  const compact = $derived(compactViewport.current);
  const open = $derived(playlists.open);

  $effect(() => {
    const user = userId;
    return untrack(() => playlists.connect(user));
  });
  // A link opens the playlist it names.
  $effect(() => {
    const id = route.playlist;
    if (!id) return;
    untrack(() => {
      playlists.select(id);
      if (!playlists.open) playlists.setOpen(true);
    });
  });
  // Closed, the address stops naming a playlist, so reloading stays closed.
  $effect(() => {
    if (!open && route.playlist)
      updateFilters({ ...route, playlist: undefined });
  });

  function start(track: PlaylistTrack, fileId?: string) {
    const list = playlists.loaded;
    const queue = list && playlistQueue(list, playlists.shown, track);
    if (queue) play({ ...queue, fileId });
  }
  function leave(destination: AppRoute) {
    if (compact) playlists.setOpen(false);
    go(destination);
  }
  const page = (item: string, ancestors?: string[]): AppRoute => ({
    section: 'Music',
    settings: route.settings,
    item,
    ancestors,
  });
</script>

{#snippet sidebar(onClose?: () => void)}
  <PlaylistSidebar
    play={(track) => start(track)}
    details={(track) => (detailed = track)}
    openAlbum={(track) =>
      leave(
        page(track.parent_id!, track.artist_id ? [track.artist_id] : undefined),
      )}
    openArtist={(track) => leave(page(track.artist_id!))}
    onSelect={(id) => {
      playlists.select(id);
      updateFilters({ ...route, playlist: id });
    }}
    {onClose}
  />
{/snippet}

{#if compact}
  <Drawer
    {open}
    side="right"
    aria-label="Music playlists"
    class="fixed inset-x-0 top-(--mobile-player-height) z-60 h-[calc(100dvh-var(--mobile-player-height,0px)-var(--music-dock-height,0px))] w-full max-w-none desktop-shell:h-[calc(100dvh-var(--mobile-player-height,0px)-var(--music-dock-height,0px)-32px)]"
    onClose={() => playlists.setOpen(false)}
  >
    {@render sidebar(() => playlists.setOpen(false))}
  </Drawer>
{:else}
  <div
    data-youtube-watchlist-frame
    data-page-slide
    class={`relative z-20 shrink-0 self-start overflow-visible ${open ? '' : 'pointer-events-none'}`}
    aria-hidden={!open}
  >
    <div
      data-youtube-watchlist-scale
      data-sidebar-resize="x"
      data-sidebar-resize-zoom
      class="absolute top-0 right-0"
    >
      {@render sidebar()}
    </div>
  </div>
{/if}
{#if detailed}<TrackDetails
    track={detailed}
    {admin}
    play={(fileId) => start(detailed!, fileId)}
    close={() => (detailed = null)}
    changed={() => void playlists.load(true)}
  />{/if}
