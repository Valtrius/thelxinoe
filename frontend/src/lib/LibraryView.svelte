<script lang="ts">
  import { revisions } from './server-events.svelte';
  import Notice from './ui/Notice.svelte';
  import { Folder, RefreshCw, Ellipsis } from '@lucide/svelte';
  import { DropdownMenu } from 'bits-ui';
  import { api } from './api';
  import { display } from './display.svelte';
  import { navigationCache } from './navigation-cache';
  import { inBackground } from './page-readiness';
  import { activity, byMedia } from './activity';
  import {
    attention,
    attentionErrors,
    acknowledgeAttention,
  } from './attention';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LibraryCard from './ui/LibraryCard.svelte';
  import MediaSkeleton from './ui/MediaSkeleton.svelte';
  import Skeleton from './ui/Skeleton.svelte';
  import TitleMenu from './title/TitleMenu.svelte';
  import { showToast } from './providers/toasts';
  import { loadRoots, scanRoot } from './title/catalog';
  import type { TitleStatus } from './title/status';
  import { untrack, onDestroy } from 'svelte';
  import Button from './ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from './ui/MenuBackdrop.svelte';
  import { emptyClass } from './ui/styles';
  import LibraryToolbar from './LibraryToolbar.svelte';
  import { appearance, updateAppearance } from './appearance';
  import {
    filtering,
    libraryViewKey,
    parseLibraryView,
    sortDetail,
    viewQuery,
    type LibraryCounts,
    type LibraryView,
    type SortValues,
    unfiltered,
  } from './library-view';
  let { domain, admin, opened, searchQuery, collectionId, filtersChanged } =
    $props<{
      domain: 'Movies' | 'Shows';
      admin: boolean;
      /** A card opens its title page. */
      opened: (id: string) => void;
      searchQuery?: string;
      collectionId?: string;
      filtersChanged: (query: string, collection: string) => void;
    }>();
  type Item = SortValues & {
    id: string;
    kind: string;
    title: string;
    year: number | null;
    available: boolean;
    watched: boolean;
    metadata: Record<string, unknown>;
    artwork_url?: string;
    overview?: string;
    virtual?: boolean;
    status?: TitleStatus | null;
    ended?: boolean;
  };
  type Page = {
    items: Item[];
    total?: number;
    counts?: LibraryCounts;
    genres?: string[];
  };
  let items = $state<Item[]>([]),
    total = $state(0),
    counts = $state<LibraryCounts>(),
    genres = $state<string[]>([]),
    error = $state(''),
    busy = $state(true);
  // The grid reads its list a page at a time as the reader nears the end.
  const pageSize = 120;
  // The list `items` holds, and its next page loading or failing.
  let listed = '',
    extending = $state(false),
    pageError = $state('');
  function show(path: string, result: Page) {
    listed = path;
    items = result.items;
    total = result.total ?? result.items.length;
    counts = result.counts;
    genres = result.genres ?? [];
  }
  // Each user keeps their sorting and filters.
  const viewKey = $derived(libraryViewKey(domain));
  const shows = $derived(domain === 'Shows');
  const view = $derived<LibraryView>(
    parseLibraryView($appearance.provider_preferences[viewKey], shows),
  );
  const viewParameters = $derived(viewQuery(view, shows));
  const filtered = $derived(filtering(view, shows));
  function viewChanged(next: LibraryView) {
    updateAppearance({
      provider_preferences: { [viewKey]: JSON.stringify(next) },
    });
  }
  let collections = $state<{ id: number; name: string; count: number }[]>([]),
    collection = $state('');
  let generation = 0;
  const activityOf = $derived(byMedia($activity));
  const attentionOf = $derived(byMedia($attention));
  let search = $state('');
  let searchTimer: ReturnType<typeof setTimeout>;
  onDestroy(() => {
    clearTimeout(searchTimer);
    generation++;
  });
  function searchChanged() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => filtersChanged(search, collection), 250);
  }
  async function load() {
    const request = ++generation;
    busy = true;
    extending = false;
    error = '';
    pageError = '';
    try {
      const path = `/catalog?kind=${domain === 'Movies' ? 'movie' : 'show'}&q=${encodeURIComponent(search)}${collection ? `&collection=${collection}` : ''}${viewParameters ? `&${viewParameters}` : ''}`;
      const list = navigationCache.read(
        'catalog',
        path,
        // Reading a list again keeps the pages it shows.
        () =>
          api<Page>(
            `${path}&limit=${Math.max(pageSize, listed === path ? items.length : 0)}`,
          ),
        (result) => {
          if (request === generation) show(path, result);
        },
      );
      const groups =
        domain === 'Movies'
          ? navigationCache.read(
              'catalog',
              '/catalog/collections',
              () => api<{ items: typeof collections }>('/catalog/collections'),
              (result) => {
                if (request === generation) collections = result.items;
              },
            )
          : Promise.resolve();
      await Promise.all([list, groups]);
    } catch (e) {
      if (request === generation) error = String(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  async function more() {
    if (busy || extending || pageError || items.length >= total) return;
    const request = generation,
      path = listed;
    extending = true;
    try {
      const page = await inBackground(() =>
        api<Page>(`${path}&offset=${items.length}&limit=${pageSize}`),
      );
      if (request !== generation) return;
      const known = new Set(items.map((item) => item.id));
      const added = page.items.filter((item) => !known.has(item.id));
      show(path, {
        ...page,
        items: [...items, ...added],
        // A page with nothing new ends the list until it is read again.
        total: added.length ? page.total : items.length,
      });
      navigationCache.update('catalog', path, { items, total, counts, genres });
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
  $effect(() => {
    void domain;
    const query = searchQuery ?? '',
      group = collectionId ?? '';
    untrack(() => {
      search = query;
      collection = group;
      void load();
    });
  });
  let observedView: string | undefined;
  $effect(() => {
    const next = viewParameters;
    untrack(() => {
      if (observedView !== undefined && observedView !== next) void load();
      observedView = next;
    });
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
  let actionsOpen = $state(false);
  const menuItemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] compact:h-11 compact:text-[13px]';
  async function scanLibrary() {
    try {
      const roots = (await loadRoots()).filter(
        (root) => root.kind === domain.toLowerCase(),
      );
      await Promise.all(roots.map((root) => scanRoot(root.id)));
      showToast({
        key: `scan:${domain}`,
        tone: roots.length ? 'success' : 'warning',
        title: domain,
        message: roots.length
          ? `Scanning ${roots.length} library ${roots.length === 1 ? 'folder' : 'folders'}.`
          : 'No accessible media folders have been detected yet.',
      });
    } catch (e) {
      showToast({
        key: `scan:${domain}`,
        tone: 'error',
        title: domain,
        message: String(e),
      });
    }
  }
</script>

<div class="mb-5 flex flex-wrap items-center gap-1.5">
  <LibraryToolbar
    {domain}
    bind:search
    {searchChanged}
    {view}
    {viewChanged}
    {counts}
    {genres}
    {collections}
    bind:collection
    collectionChanged={() => filtersChanged(search, collection)}
  />{#if busy && !items.length}<Skeleton class="ml-1.5 w-16" />{:else}<span
      class="ml-1.5 text-muted">{total} items</span
    >{/if}
  {#if admin}<div class="ml-auto">
      <DropdownMenu.Root bind:open={actionsOpen}>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}<Button
              {...props}
              variant="secondary"
              size="icon"
              aria-label={`${domain} library actions`}
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
{#if items.length}<MediaGrid
    revision={items.map((i) => i.id).join(',')}
    label={domain}
  >
    {#each items as item (item.id)}<LibraryCard
        {item}
        activity={activityOf(item.id)}
        attention={attentionOf(item.id)}
        attentionError={attentionOf(item.id)
          .map((entry) => $attentionErrors[entry.id])
          .filter(Boolean)
          .join('\n')}
        onSeen={() => {
          for (const entry of attentionOf(item.id))
            void acknowledgeAttention(entry);
        }}
        open={() => opened(item.id)}
        timezone={display.timezone}
        detail={sortDetail(view.sort, item, display.timezone)}
        >{#snippet menu(
          card,
        )}{#if !item.virtual || (admin && item.kind === 'movie')}<TitleMenu
              bind:open={card.open}
              id={item.virtual ? null : item.id}
              kind={item.kind}
              title={item.title}
              {admin}
              radarr={item.virtual ? item.id : null}
              variant="card"
              changed={() => void load()}
            />{/if}{/snippet}</LibraryCard
      >{/each}
  </MediaGrid>
  {#if items.length < total}
    <!-- Mounted again after each read, so a list still short of the screen keeps loading. -->
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
      >{:else if extending}<MediaSkeleton
        label={`Loading more ${domain.toLowerCase()}`}
      />{/if}
  {/if}
{:else if busy}<MediaSkeleton
    label={`Loading ${domain.toLowerCase()}`}
  />{:else if !error}<section class={emptyClass}>
    <Folder size={42} />
    <h2>
      {search || filtered || collection ? 'No matches' : 'No media yet'}
    </h2>
    {#if search}<p>Try a different search.</p>{:else if filtered}<Button
        variant="secondary"
        size="form"
        onclick={() => viewChanged(unfiltered(view))}>Clear filters</Button
      >{:else if !collection}<p>Imported media will appear here.</p>{/if}
  </section>{/if}
