<script lang="ts">
  import { untrack } from 'svelte';
  import type { User } from '../api';
  import { readRoute, type AppRoute } from '../navigation';
  import type { MediaChoice } from '../playback';
  import { loadItem, type CatalogItem } from '../title/catalog';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import MusicHome from './MusicHome.svelte';
  import ArtistPage from './ArtistPage.svelte';
  import AlbumPage from './AlbumPage.svelte';

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
  const listRoute = (view: string | undefined, query: string): AppRoute => ({
    ...home,
    view,
    query: query || undefined,
  });
  const openArtist = (id: string) => go({ ...home, item: id });
  const openAlbum = (id: string, artist?: string | null) =>
    go({ ...home, item: id, ancestors: artist ? [artist] : undefined });
</script>

{#if !route.item}<MusicHome
    view={route.view}
    query={route.query}
    {admin}
    userId={user.id}
    {play}
    {openArtist}
    {openAlbum}
    viewChanged={(view, query) => go(listRoute(view, query), true)}
    filtersChanged={(query) => updateFilters(listRoute(route.view, query))}
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
      backLabel={backLabel('Music')}
      back={() => leave(home)}
    />{/key}{:else}{#key item.id}<AlbumPage
      album={item}
      {artistName}
      {admin}
      userId={user.id}
      {play}
      {focus}
      openArtist={item.parent_id
        ? () => openArtist(item!.parent_id!)
        : undefined}
      backLabel={backLabel(artistName || 'Music', item.parent_id)}
      back={() =>
        leave(item?.parent_id ? { ...home, item: item.parent_id } : home)}
    />{/key}{/if}
