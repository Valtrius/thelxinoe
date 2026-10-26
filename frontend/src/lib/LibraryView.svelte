<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import Notice from './ui/Notice.svelte';
  import { Folder, RefreshCw, ArrowLeft, Ellipsis } from '@lucide/svelte';
  import { DropdownMenu } from 'bits-ui';
  import { api } from './api';
  import { navigationCache } from './navigation-cache';
  import { activity } from './activity';
  import { LatestRequest } from './latest-request';
  import {
    attention,
    attentionErrors,
    acknowledgeAttention,
  } from './attention';
  import MediaGrid from './ui/MediaGrid.svelte';
  import LibraryCard from './ui/LibraryCard.svelte';
  import MediaSkeleton from './ui/MediaSkeleton.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Skeleton from './ui/Skeleton.svelte';
  import MediaActions from './MediaActions.svelte';
  import MediaOperations from './MediaOperations.svelte';
  import TitleMenu from './title/TitleMenu.svelte';
  import { showToast } from './providers/toasts';
  import { loadRoots, scanRoot } from './title/catalog';
  import type { TitleStatus } from './title/status';
  import { untrack, onDestroy } from 'svelte';
  import type { MediaChoice } from './playback';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { emptyClass, rowClass } from './ui/styles';
  import LibraryToolbar from './LibraryToolbar.svelte';
  import { appearance, updateAppearance } from './appearance';
  import {
    filtering,
    libraryViewKey,
    parseLibraryView,
    viewQuery,
    type LibraryCounts,
    type LibraryView,
    unfiltered,
  } from './library-view';
  let {
    domain,
    admin,
    revision = 0,
    play,
    userId,
    timezone,
    focusId,
    ancestors,
    selectedChanged,
    searchQuery,
    collectionId,
    filtersChanged,
  } = $props<{
    domain: string;
    admin: boolean;
    revision?: number;
    play: (choice: MediaChoice) => void;
    userId: string;
    timezone: string;
    focusId?: string;
    ancestors?: string[];
    selectedChanged?: (id?: string, ancestors?: string[]) => void;
    searchQuery?: string;
    collectionId?: string;
    filtersChanged?: (query: string, collection: string) => void;
  }>();
  type Item = {
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
  // Movies and Shows open a title page; Music keeps its track panel here.
  const titles = $derived(domain !== 'Music');
  let items = $state<Item[]>([]),
    total = $state(0),
    counts = $state<LibraryCounts>(),
    genres = $state<string[]>([]),
    error = $state(''),
    busy = $state(true);
  // Movies and Shows keep each user's sorting and filters.
  const viewKey = $derived(libraryViewKey(domain));
  const view = $derived<LibraryView | undefined>(
    titles
      ? parseLibraryView($appearance.provider_preferences[viewKey])
      : undefined,
  );
  const shows = $derived(domain === 'Shows');
  const viewParameters = $derived(view ? viewQuery(view, shows) : '');
  const filtered = $derived(!!view && filtering(view, shows));
  function viewChanged(next: LibraryView) {
    updateAppearance({
      provider_preferences: { [viewKey]: JSON.stringify(next) },
    });
  }
  let collections = $state<{ id: number; name: string; count: number }[]>([]),
    collection = $state('');
  let breadcrumbs = $state<{ id: string; title: string; kind: string }[]>([]),
    selected = $state<Item | null>(null),
    details = $state<{
      files: {
        id: string;
        edition: string;
        present: boolean;
        probe: { format?: { duration?: string } };
      }[];
      local_trailers?: { file_id: string }[];
    } | null>(null);
  let generation = 0;
  const detailRequests = new LatestRequest();
  let search = $state('');
  let searchTimer: ReturnType<typeof setTimeout>;
  onDestroy(() => {
    clearTimeout(searchTimer);
    generation++;
    detailRequests.invalidate();
  });
  function searchChanged() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      if (filtersChanged) filtersChanged(search, collection);
      else void load();
    }, 250);
  }
  async function load(refreshDetails = true) {
    const request = ++generation;
    busy = true;
    error = '';
    try {
      const parent = breadcrumbs.at(-1)?.id;
      const query = parent
        ? `parent=${parent}`
        : `kind=${domain === 'Movies' ? 'movie' : domain === 'Shows' ? 'show' : 'artist'}`;
      const path = `/catalog?${query}&q=${encodeURIComponent(search)}${collection ? `&collection=${collection}` : ''}${!parent && viewParameters ? `&${viewParameters}` : ''}`;
      const list = navigationCache.read(
        'catalog',
        path,
        () =>
          api<{
            items: Item[];
            total?: number;
            counts?: LibraryCounts;
            genres?: string[];
          }>(path),
        (result) => {
          if (request === generation) {
            items = result.items;
            total = result.total ?? result.items.length;
            counts = result.counts;
            genres = result.genres ?? [];
            busy = false;
          }
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
      if (request !== generation) return;
      if (refreshDetails && selected) await loadDetails(selected.id);
    } catch (e) {
      if (request === generation) error = String(e);
    } finally {
      if (request === generation) busy = false;
    }
  }
  $effect(() => {
    void domain;
    const parents: string[] = ancestors ?? [];
    const id = focusId,
      query = searchQuery ?? '',
      group = collectionId ?? '';
    untrack(() => {
      generation++;
      busy = true;
      closeDetails();
      breadcrumbs = [];
      search = query;
      collection = group;
      error = '';
      const current = detailRequests.begin();
      void (async () => {
        try {
          const ancestry = await Promise.all(
            parents.map((parent) => api<Item>(`/catalog/${parent}`)),
          );
          if (!current()) return;
          breadcrumbs = ancestry.map(({ id, title, kind }) => ({
            id,
            title,
            kind,
          }));
          if (id) {
            const item = await api<Item & NonNullable<typeof details>>(
              `/catalog/${id}`,
            );
            if (!current()) return;
            if (['movie', 'episode', 'track'].includes(item.kind)) {
              selected = item;
              details = item;
            } else
              breadcrumbs = [
                ...breadcrumbs,
                { id: item.id, title: item.title, kind: item.kind },
              ];
          }
          if (current()) await load(false);
        } catch (e) {
          if (current()) {
            error = String(e);
            busy = false;
          }
        }
      })();
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
    const next = revision;
    untrack(() => {
      if (observedRevision !== undefined && observedRevision !== next) {
        navigationCache.invalidate('catalog');
        void load();
      }
      observedRevision = next;
    });
  });
  async function open(item: Item) {
    if (selectedChanged) {
      selectedChanged(
        item.id,
        breadcrumbs.map((entry) => entry.id),
      );
      return;
    }
    if (['movie', 'episode', 'track'].includes(item.kind)) {
      await select(item);
    } else {
      closeDetails();
      breadcrumbs = [
        ...breadcrumbs,
        { id: item.id, title: item.title, kind: item.kind },
      ];
      await load();
    }
  }
  async function select(item: Item) {
    selected = item;
    details = null;
    error = '';
    if (selectedChanged) {
      selectedChanged(
        item.id,
        breadcrumbs.map((entry) => entry.id),
      );
      return;
    }
    await loadDetails(item.id);
  }
  function closeDetails() {
    detailRequests.invalidate();
    selected = null;
    details = null;
  }
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
  async function loadDetails(id: string) {
    const current = detailRequests.begin();
    try {
      const detail = await api<Item & NonNullable<typeof details>>(
        `/catalog/${id}`,
      );
      if (current() && selected?.id === id) {
        selected = detail;
        details = detail;
      }
    } catch (e) {
      if (current() && selected?.id === id) error = String(e);
    }
  }
</script>

{#if breadcrumbs.length}
  <div class="mb-4">
    <Button
      variant="secondary"
      size="form"
      onclick={() => {
        if (selectedChanged) {
          selectedChanged(
            breadcrumbs.at(-2)?.id,
            breadcrumbs.slice(0, -2).map((entry) => entry.id),
          );
          return;
        }
        breadcrumbs = breadcrumbs.slice(0, -1);
        void load();
      }}><ArrowLeft size={15} />{breadcrumbs.at(-1)?.title}</Button
    >
  </div>
{/if}
<div class="mb-5 flex flex-wrap items-center gap-1.5">
  <LibraryToolbar
    {domain}
    bind:search
    {searchChanged}
    view={breadcrumbs.length ? undefined : view}
    {viewChanged}
    {counts}
    {genres}
    {collections}
    bind:collection
    collectionChanged={() =>
      filtersChanged ? filtersChanged(search, collection) : void load()}
  />{#if busy && !items.length}<Skeleton class="ml-1.5 w-16" />{:else}<span
      class="ml-1.5 text-muted"
      >{items.length < total ? `${items.length} of ${total}` : items.length} items</span
    >{/if}
  {#if admin}<div class="ml-auto">
      <DropdownMenu.Root>
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
          <DropdownMenu.Content
            align="end"
            sideOffset={4}
            preventScroll={false}
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
{#if selected}<Panel>
    <SectionHeading>
      <h2>{selected.title}</h2>
      <Button
        variant="secondary"
        size="form"
        onclick={() => {
          closeDetails();
          selectedChanged?.(
            breadcrumbs.at(-1)?.id,
            breadcrumbs.slice(0, -1).map((entry) => entry.id),
          );
        }}>Close</Button
      >
    </SectionHeading>
    {#if selected.overview}<p class="text-muted">{selected.overview}</p>{/if}
    <MediaActions id={selected.id} kind={selected.kind} {userId} />
    {#if admin}{#key selected.id}<MediaOperations
          id={selected.id}
          kind={selected.kind}
          changed={() => void load()}
        />{/key}{/if}
    {#if !details && !error}<ContentSkeleton label="Loading media files" />{/if}
    {#each details?.files ?? [] as file (file.id)}<div class={rowClass}>
        <span>{file.edition || 'Original edition'}</span><span
          class="text-muted"
          >{file.probe.format?.duration
            ? `${Math.round(Number(file.probe.format.duration) / 60)} minutes`
            : 'Duration unknown'}</span
        >
        {#if file.present}<Button
            size="form"
            onclick={() =>
              play({
                id: selected!.id,
                title: selected!.title,
                fileId: file.id,
                kind: selected!.kind,
                queue:
                  selected!.kind === 'track'
                    ? items.filter((i) => i.kind === 'track' && i.available)
                    : undefined,
              })}>Play {file.edition || 'media'}</Button
          >{/if}
      </div>
    {/each}
  </Panel>{/if}
{#if items.length}<MediaGrid
    revision={items.map((i) => i.id).join(',')}
    label={domain}
  >
    {#each items as item (item.id)}<LibraryCard
        {item}
        activity={$activity.filter((entry) =>
          entry.media_ids.includes(item.id),
        )}
        attention={$attention.filter((entry) =>
          entry.media_ids.includes(item.id),
        )}
        attentionError={$attention
          .filter((entry) => entry.media_ids.includes(item.id))
          .map((entry) => $attentionErrors[entry.id])
          .filter(Boolean)
          .join('\n')}
        onSeen={() => {
          for (const entry of $attention.filter(
            (entry) =>
              entry.media_ids.includes(item.id) &&
              (entry.target !== 'music' || item.kind === 'album'),
          ))
            void acknowledgeAttention(entry);
        }}
        open={() => void open(item)}
        details={titles ? undefined : () => void select(item)}
        timezone={titles ? timezone : undefined}
        play={['movie', 'episode', 'track'].includes(item.kind) &&
        item.available
          ? () =>
              play({
                id: item.id,
                title: item.title,
                kind: item.kind,
                queue:
                  item.kind === 'track'
                    ? items.filter((i) => i.kind === 'track' && i.available)
                    : undefined,
              })
          : undefined}
        >{#snippet menu()}{#if titles && !item.virtual}<TitleMenu
              id={item.id}
              kind={item.kind}
              title={item.title}
              {admin}
              variant="card"
              changed={() => void load()}
            />{/if}{/snippet}</LibraryCard
      >{/each}
  </MediaGrid>{:else if busy}<MediaSkeleton
    label={`Loading ${domain.toLowerCase()}`}
    shape={domain === 'Music' ? 'square' : 'poster'}
  />{:else if !error}<section class={emptyClass}>
    <Folder size={42} />
    <h2>
      {search || filtered || collection ? 'No matches' : 'No media yet'}
    </h2>
    {#if search}<p>Try a different search.</p>{:else if filtered}<Button
        variant="secondary"
        size="form"
        onclick={() => view && viewChanged(unfiltered(view))}
        >Clear filters</Button
      >{:else if !collection}<p>Imported media will appear here.</p>{/if}
  </section>{/if}
