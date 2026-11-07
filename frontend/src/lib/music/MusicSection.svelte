<script lang="ts">
  import { untrack } from 'svelte';
  import type { User } from '../api';
  import { appearance, updateAppearance } from '../appearance';
  import { createCardGridWheelHandler } from '../card-grid-wheel';
  import { scaleYoutubeMediaScope } from '../card-grid-zoom';
  import { createLayoutMotion } from '../layout-motion';
  import { readRoute, type AppRoute } from '../navigation';
  import type { MediaChoice } from '../playback';
  import { compactViewport } from '../responsive';
  import { loadItem, type CatalogItem } from '../title/catalog';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import BackButton from './BackButton.svelte';
  import MusicHome from './MusicHome.svelte';
  import MusicToolbar from './MusicToolbar.svelte';
  import PlaylistToggle from './PlaylistToggle.svelte';
  import ArtistPage from './ArtistPage.svelte';
  import AlbumPage from './AlbumPage.svelte';
  import PlaylistPanel from './PlaylistPanel.svelte';
  import { listSort, listView } from './list-view';
  import { playlists } from './playlists.svelte';

  let { route, user, play, go, updateFilters } = $props<{
    route: AppRoute;
    user: User;
    play: (choice: MediaChoice) => void;
    go: (route: AppRoute, replace?: boolean) => void;
    updateFilters: (route: AppRoute, replace?: boolean) => void;
  }>();
  const admin = $derived(user.role === 'admin');
  const home = $derived<AppRoute>({
    section: 'Music',
    settings: route.settings,
  });
  // An artist or album page reads its item; a track opens its album.
  let item = $state<CatalogItem | null>(null),
    artistName = $state(''),
    focus = $state<string | undefined>(),
    error = $state(''),
    attempt = $state(0);
  $effect(() => {
    const id = route.item;
    void attempt;
    untrack(() => {
      item = null;
      error = '';
      artistName = '';
      focus = undefined;
      if (id) void open(id);
    });
  });
  async function open(id: string) {
    try {
      let found = await loadItem(id);
      if (route.item !== id) return;
      if (found.kind === 'track' && found.parent_id) {
        focus = found.id;
        found = await loadItem(found.parent_id);
      }
      if (found.kind === 'album' && found.parent_id)
        artistName = (await loadItem(found.parent_id)).title;
      if (route.item === id) item = found;
    } catch (e) {
      if (route.item === id) error = String(e);
    }
  }
  // The page this one was opened from, if it was opened inside the app.
  const opener = $derived.by(() => {
    void route;
    const from: unknown = history.state?.from;
    return typeof from === 'string'
      ? readRoute(new URL(from, location.href))
      : undefined;
  });
  // Back returns to that page, restoring its scroll, or else up to the parent.
  function leave(parent: AppRoute) {
    if (opener) history.back();
    else go(parent);
  }
  function backLabel(parent: string, parentId?: string | null) {
    if (!opener || (parentId && opener.item === parentId)) return parent;
    if (opener.section !== 'Music') return opener.section;
    return opener.item ? undefined : 'Music';
  }
  // An album returns to its artist, and anything else to the lists.
  const parent = $derived(
    item?.kind === 'artist' ? undefined : item?.parent_id,
  );
  const view = $derived(listView(route.view, $appearance.provider_preferences));
  const sort = $derived(listSort(view, $appearance.provider_preferences));
  let counts = $state<Partial<Record<string, number>>>({}),
    listing = $state(true);
  const listRoute = (view: string | undefined, query: string): AppRoute => ({
    ...home,
    view,
    query: query || undefined,
  });
  const openArtist = (id: string) => go({ ...home, item: id });
  const openAlbum = (id: string, artist?: string | null) =>
    go({ ...home, item: id, ancestors: artist ? [artist] : undefined });
  const motion = createLayoutMotion();
  const zoomWheel = createCardGridWheelHandler({
    kind: 'poster',
    anchorAttribute: 'data-layout-key',
    motion,
    getColumns: () => $appearance.poster_columns,
    setColumns: (poster_columns) => updateAppearance({ poster_columns }),
  });
</script>

{#snippet page()}
  {#if !route.item}<MusicHome
      {view}
      {sort}
      query={route.query}
      {admin}
      bind:counts
      bind:busy={listing}
      {play}
      {openArtist}
      {openAlbum}
    />{:else if error}<Notice variant="error" role="alert"
      >{error}<Button variant="ghost" size="sm" onclick={() => attempt++}
        >Reload</Button
      ></Notice
    >{:else if !item}<ContentSkeleton
      label="Loading details"
      variant="detail"
    />{:else if item.kind === 'artist'}{#key item.id}<ArtistPage
        artist={item}
        {admin}
        {play}
        openAlbum={(id) => openAlbum(id, item?.id)}
      />{/key}{:else}{#key item.id}<AlbumPage
        album={item}
        {artistName}
        {admin}
        {play}
        {focus}
        openArtist={item.parent_id
          ? () => openArtist(item!.parent_id!)
          : undefined}
      />{/key}{/if}
{/snippet}

<div
  data-sidebar-resize="x"
  class="flex h-full min-h-0 flex-col overflow-hidden"
>
  <div
    data-music-toolbar
    data-page-scale
    class="relative z-30 mx-3 mt-3 flex shrink-0 items-start gap-1.5 pb-2"
  >
    <span
      data-sidebar-resize="x"
      aria-hidden="true"
      class="pointer-events-none absolute right-0 bottom-0 left-0 h-px bg-line"
    ></span>
    {#if !route.item}<MusicToolbar
        {view}
        {sort}
        query={route.query}
        {counts}
        busy={listing}
        {admin}
        viewChanged={(view, query) => go(listRoute(view, query), true)}
        filtersChanged={(query) => updateFilters(listRoute(route.view, query))}
      />{:else if item}<BackButton
        label={backLabel(parent ? artistName || 'Music' : 'Music', parent)}
        back={() => leave(parent ? { ...home, item: parent } : home)}
      />{/if}
    <PlaylistToggle />
  </div>
  <div
    use:scaleYoutubeMediaScope={{
      kind: 'poster',
      columns: compactViewport.current ? 3 : $appearance.poster_columns,
      sidebarOpen: !compactViewport.current && playlists.open,
    }}
    data-sidebar-media-scope
    class="flex min-h-0 min-w-0 flex-1"
  >
    <div
      use:motion.connect
      onwheel={zoomWheel}
      data-sidebar-resize="x"
      data-sidebar-resize-origin
      data-feed-scroll
      data-feed-content
      data-page-scale
      data-music-scroll
      class="relative min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto pt-3 pr-0.5 pb-6 pl-3 [overflow-anchor:none] [scrollbar-gutter:stable] compact:pr-3"
    >
      <div class="min-w-0" data-page-region="music-page">
        {@render page()}
      </div>
    </div>
    <PlaylistPanel
      {route}
      userId={user.id}
      {admin}
      {play}
      {go}
      {updateFilters}
    />
  </div>
</div>
