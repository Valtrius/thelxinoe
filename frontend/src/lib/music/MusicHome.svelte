<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { DropdownMenu, Popover } from 'bits-ui';
  import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    Ellipsis,
    Music,
    Play,
    RefreshCw,
    Search,
  } from '@lucide/svelte';
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
  import { appearance, updateAppearance } from '../appearance';
  import { showToast } from '../providers/toasts';
  import { loadRoots, scanRoot } from '../title/catalog';
  import type { MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import MediaGrid from '../ui/MediaGrid.svelte';
  import LibraryCard from '../ui/LibraryCard.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import Skeleton from '../ui/Skeleton.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import { emptyClass, eyebrowTextClass, toolbarMenuClass } from '../ui/styles';
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

  type View = 'artists' | 'albums' | 'tracks';
  type Sort = 'title' | 'year' | 'added';
  type Order = 'asc' | 'desc';
  let {
    view: routeView,
    query,
    admin,
    userId,
    play,
    openArtist,
    openAlbum,
    viewChanged,
    filtersChanged,
  } = $props<{
    view?: string;
    query?: string;
    admin: boolean;
    userId: string;
    play: (choice: MediaChoice) => void;
    openArtist: (id: string) => void;
    openAlbum: (id: string, artist?: string | null) => void;
    viewChanged: (view: View, query: string) => void;
    filtersChanged: (view: View, query: string) => void;
  }>();
  const views: [View, string, string][] = [
    ['artists', 'Artists', 'artist'],
    ['albums', 'Albums', 'album'],
    ['tracks', 'Tracks', 'track'],
  ];
  const view = $derived<View>(
    views.some(([value]) => value === routeView)
      ? (routeView as View)
      : 'artists',
  );
  const kind = $derived(views.find(([value]) => value === view)![2]);
  const sorts: Record<View, [Sort, string][]> = {
    artists: [
      ['title', 'Name'],
      ['added', 'Date added'],
    ],
    albums: [
      ['title', 'Title'],
      ['year', 'Year'],
      ['added', 'Date added'],
    ],
    tracks: [
      ['title', 'Title'],
      ['added', 'Date added'],
    ],
  };
  // Each list keeps the user's sort.
  const sortKey = 'music-sort';
  const saved = $derived.by(() => {
    try {
      return JSON.parse(
        $appearance.provider_preferences[sortKey] ?? '{}',
      ) as Partial<Record<View, { sort: Sort; order: Order }>>;
    } catch {
      return {};
    }
  });
  const sort = $derived(
    saved[view] && sorts[view].some(([value]) => value === saved[view]!.sort)
      ? saved[view]!
      : { sort: 'title' as Sort, order: 'asc' as Order },
  );
  function sortChanged(next: { sort: Sort; order: Order }) {
    updateAppearance({
      provider_preferences: {
        [sortKey]: JSON.stringify({ ...saved, [view]: next }),
      },
    });
  }
  let search = $state(untrack(() => query ?? ''));
  let searchTimer: ReturnType<typeof setTimeout>;
  // The route echoing a search sent while typing leaves the field alone.
  let sent: string | undefined;
  $effect(() => {
    const next = query ?? '';
    untrack(() => {
      if (next !== sent) search = next;
    });
  });
  function searchChanged() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      sent = search;
      filtersChanged(view, search);
    }, 250);
  }
  onDestroy(() => {
    clearTimeout(searchTimer);
    generation++;
  });

  const pageSize = 120;
  let items = $state<MusicItem[]>([]),
    listedKind = $state(''),
    total = $state(0),
    counts = $state<Partial<Record<string, number>>>({}),
    busy = $state(true),
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
      { root: node.closest('.workspace-scroll'), rootMargin: '150% 0px' },
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

  let sortOpen = $state(false),
    actionsOpen = $state(false);
  const menuItemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] compact:h-11 compact:text-[13px]';
  async function scanLibrary() {
    try {
      const roots = (await loadRoots()).filter((root) => root.kind === 'music');
      await Promise.all(roots.map((root) => scanRoot(root.id)));
      showToast({
        key: 'scan:Music',
        tone: roots.length ? 'success' : 'warning',
        title: 'Music',
        message: roots.length
          ? `Scanning ${roots.length} library ${roots.length === 1 ? 'folder' : 'folders'}.`
          : 'No accessible media folders have been detected yet.',
      });
    } catch (e) {
      showToast({
        key: 'scan:Music',
        tone: 'error',
        title: 'Music',
        message: String(e),
      });
    }
  }
  // The buttons answer the click, before the route reaches the next list.
  let selected = $derived<View>(view);
  const viewChoices = $derived(
    views.map(([value, label, countKind]) => ({
      value,
      label,
      count: counts[countKind],
    })),
  );
</script>

<div class="mb-5 flex flex-wrap items-center gap-1.5">
  <ExclusiveChoiceGroup
    choices={viewChoices}
    value={selected}
    ariaLabel="Music lists"
    tall
    onChange={(next) => {
      selected = next;
      clearTimeout(searchTimer);
      viewChanged(next, search);
    }}
  />
  <div
    class="relative flex h-9 min-w-45 max-w-90 flex-1 items-center border border-line bg-surface focus-within:border-line-strong"
  >
    <Search
      class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted"
    />
    <input
      bind:value={search}
      oninput={searchChanged}
      maxlength="200"
      placeholder={`Search ${view}`}
      aria-label={`Search ${view}`}
      class="h-full min-w-0 flex-1 bg-transparent pr-3 pl-9 text-sm outline-none placeholder:text-muted"
    />
  </div>
  <div class="relative z-30 compact:static">
    <Popover.Root bind:open={sortOpen}>
      <Popover.Trigger>
        {#snippet child({ props })}<Button
            {...props}
            size="icon"
            variant={sortOpen ? 'secondary' : 'ghost'}
            aria-label={`Sort ${view}`}
            title={`Sort ${view}`}><ArrowUpDown class="size-4" /></Button
          >{/snippet}
      </Popover.Trigger>
      {#if sortOpen}<MenuBackdrop close={() => (sortOpen = false)} />{/if}
      <Popover.ContentStatic
        role="dialog"
        aria-label={`Sort ${view}`}
        onInteractOutside={ignoreBackdrop}
        class={`absolute top-11 left-0 z-50 w-64 border border-line-strong bg-surface-strong p-3 shadow-xl ${toolbarMenuClass}`}
      >
        <p class={`${eyebrowTextClass} mb-2`}>SORT BY</p>
        {#each sorts[view] as [value, label] (value)}
          <div
            class="flex items-center gap-1 border-t border-line py-1.5 first:border-0"
          >
            <span class="flex-1 text-sm">{label}</span>
            {#each ['asc', 'desc'] as const as order (order)}
              <button
                type="button"
                class={`grid size-8 place-items-center border ${sort.sort === value && sort.order === order ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
                aria-label={`${label} ${order === 'asc' ? 'ascending' : 'descending'}`}
                aria-pressed={sort.sort === value && sort.order === order}
                onclick={() => sortChanged({ sort: value, order })}
                >{#if order === 'asc'}<ArrowUp size={14} />{:else}<ArrowDown
                    size={14}
                  />{/if}</button
              >
            {/each}
          </div>
        {/each}
      </Popover.ContentStatic>
    </Popover.Root>
  </div>
  {#if counts[kind] !== undefined}<span class="ml-1.5 text-muted"
      >{plural(counts[kind], kind)}</span
    >{:else if busy}<Skeleton class="ml-1.5 w-16" />{/if}
  {#if admin}<div class="ml-auto">
      <DropdownMenu.Root bind:open={actionsOpen}>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}<Button
              {...props}
              variant="secondary"
              size="icon"
              aria-label="Music library actions"
              title="Library actions"><Ellipsis size={16} /></Button
            >{/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          {#if actionsOpen}<MenuBackdrop
              close={() => (actionsOpen = false)}
            />{/if}
          <DropdownMenu.Content
            align="end"
            sideOffset={4}
            preventScroll={false}
            onInteractOutside={ignoreBackdrop}
            class="z-50 grid min-w-48 border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
          >
            <DropdownMenu.Item
              class={menuItemClass}
              onSelect={() => void scanLibrary()}
              ><RefreshCw />Scan library</DropdownMenu.Item
            >
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>{/if}
</div>
{#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
<div data-page-region="music">
  {#if shown.length}
    {#if view === 'tracks'}<TrackList
        tracks={shown}
        {userId}
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
    {userId}
    {admin}
    play={(fileId) => void playTrack(detailed!, fileId)}
    close={() => (detailed = null)}
    changed={() => void load()}
  />{/if}
