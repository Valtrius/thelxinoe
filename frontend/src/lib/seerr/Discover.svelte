<script lang="ts">
  import { onMount, onDestroy, tick, untrack } from 'svelte';
  import { DropdownMenu } from 'bits-ui';
  import {
    Search,
    X,
    LoaderCircle,
    ArrowRight,
    RefreshCw,
    Ellipsis,
    RotateCcw,
    Ban,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { forgetSeerrStatus, seerrStatus, seerrTitle } from './status';
  import { inBackground } from '../page-readiness';
  import { navigationCache } from '../navigation-cache';
  import { attention, acknowledgeAttention } from '../attention';
  import AttentionDot from '../ui/AttentionDot.svelte';
  import Button from '../ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  import MediaRow from './MediaRow.svelte';
  import MediaSkeleton from '../ui/MediaSkeleton.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import PosterCard from './PosterCard.svelte';
  import TitlePage from '../title/TitlePage.svelte';
  import type { MediaChoice } from '../playback';
  import {
    artwork,
    backdropSet,
    posterSet,
    title,
    kind,
    year,
    type Media,
    type MediaKind,
    type Results,
    type MediaRequest,
  } from './types';
  let {
    user,
    settings,
    detail,
    showRequests = false,
    openMedia,
    back,
    searchQuery,
    searchChanged,
    requestsChanged,
    play,
    revision = 0,
  } = $props<{
    user: User;
    play: (choice: MediaChoice) => void;
    revision?: number;
    settings: () => void;
    detail?: { kind: MediaKind; id: number };
    showRequests?: boolean;
    openMedia: (detail: { kind: MediaKind; id: number }) => void;
    back: () => void;
    searchQuery?: string;
    searchChanged: (query: string) => void;
    requestsChanged: (requests: boolean) => void;
  }>();
  type Feed = {
    id: string;
    label: string;
    items: Media[];
    error: string;
    loading: boolean;
    requested: boolean;
    // Whether the row is on or near the screen.
    near: boolean;
  };
  const feed = (id: string, label: string): Feed => ({
    id,
    label,
    items: [],
    error: '',
    loading: true,
    requested: false,
    near: false,
  });
  let feeds = $state<Feed[]>([
    feed('trending', 'Trending'),
    feed('movies', 'Popular movies'),
    feed('tv', 'Popular series'),
    feed('upcoming', 'Upcoming movies'),
    feed('upcoming-tv', 'Upcoming series'),
  ]);
  // Seerr answered ready, so rows may load.
  let feedsReady = $state(false);
  let tab = $state('discover'),
    query = $state(''),
    results = $state<Media[]>([]),
    page = $state(1),
    pages = $state(1),
    total = $state(0),
    searching = $state(false),
    configured = $state(true),
    error = $state('');
  const selected = $derived(detail ?? null);
  let requests = $state<(MediaRequest & { detail?: Media })[]>([]),
    requestPages = $state(1),
    requestPage = $state(1),
    requestLoading = $state(false),
    actionBusy = $state<number | null>(null),
    requestMenu = $state<number | null>(null);
  let searchTimer: ReturnType<typeof setTimeout>,
    searchGeneration = 0,
    requestGeneration = 0,
    requestedPage = 1,
    active = true;
  let browseScroll = 0;
  const requestAttention = $derived(
    $attention
      .filter((item) => item.id.startsWith('seerr:'))
      .map((item) => `${item.id}:${item.revision}`)
      .join('\n'),
  );
  // The attention the shown list was read for, and what the list acknowledged
  // since. Its own acknowledgements are no reason to read it again.
  let readAttention: string[] | undefined;
  let acknowledged: string[] = [];
  $effect(() => {
    if (tab !== 'requests' || selected || !configured) {
      readAttention = undefined;
      return;
    }
    // Track source revisions without restarting loads for unrelated menu changes.
    const current = requestAttention.split('\n').filter(Boolean);
    untrack(() => {
      if (
        readAttention &&
        current.every((entry) => readAttention!.includes(entry)) &&
        readAttention.every(
          (entry) => current.includes(entry) || acknowledged.includes(entry),
        )
      )
        return;
      readAttention = current;
      acknowledged = [];
      void loadRequests(requestedPage);
    });
  });
  // Thelxinoe's own acquisition requests have no list here any more; opening
  // Requests is when their decisions are seen.
  $effect(() => {
    if (tab !== 'requests' || selected) return;
    for (const entry of $attention.filter(
      (item) => item.target === 'requests' && item.id.startsWith('request:'),
    ))
      void acknowledgeAttention(entry);
  });
  const filtered = $derived(
    results.filter(
      (item) =>
        item.mediaType !== 'person' &&
        (tab === 'movies'
          ? kind(item) === 'movie'
          : tab === 'tv'
            ? kind(item) === 'tv'
            : true),
    ),
  );
  const visibleFeeds = $derived(
    feeds.filter((f) =>
      tab === 'movies'
        ? ['movies', 'upcoming'].includes(f.id)
        : tab === 'tv'
          ? ['tv', 'upcoming-tv'].includes(f.id)
          : true,
    ),
  );
  const featured = $derived(
    tab === 'discover' && !query.trim()
      ? feeds[0].items.find((item) => artwork(item.backdropPath))
      : undefined,
  );
  const scrollWorkspace = () =>
    document.querySelector('[data-workspace-scroll]') ??
    document.querySelector('.workspace-scroll');
  $effect(() => {
    if (!selected)
      void tick().then(() => {
        if (active && !selected)
          scrollWorkspace()?.scrollTo({ top: browseScroll });
      });
  });
  $effect(() => {
    if (showRequests) tab = 'requests';
    else
      untrack(() => {
        if (tab === 'requests') tab = 'discover';
      });
  });
  $effect(() => {
    const term = searchQuery ?? '';
    untrack(() => {
      if (query !== term) {
        query = term;
        changed(false);
      }
    });
  });
  function open(item: Media) {
    openTarget({ kind: kind(item), id: item.id });
  }
  function openTarget(target: { kind: MediaKind; id: number }) {
    if (!selected) browseScroll = scrollWorkspace()?.scrollTop ?? 0;
    openMedia(target);
    scrollWorkspace()?.scrollTo({ top: 0 });
  }
  const menuItemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] compact:h-11 compact:text-[13px]';
  // The page appears with its first row. The others load as they near the
  // screen, each showing its skeleton meanwhile.
  async function load() {
    feedsReady = false;
    for (const feed of feeds) {
      feed.loading = true;
      feed.requested = false;
    }
    error = '';
    try {
      const state = await seerrStatus();
      if (!active) return;
      configured = state.configured;
      if (!configured) return;
      if (!state.ready) {
        error = 'Seerr is still completing setup. Try again shortly.';
        return;
      }
      feedsReady = true;
      // A title page, the requests or a search do not show the rows.
      await loadFeed(
        visibleFeeds[0],
        !!selected || tab === 'requests' || !!query.trim(),
      );
    } catch (caught) {
      if (active) error = String(caught);
    } finally {
      if (active && !feedsReady) for (const feed of feeds) feed.loading = false;
    }
  }
  async function loadFeed(feed: Feed, background = true) {
    if (!feedsReady || feed.requested) return;
    feed.requested = true;
    feed.loading = true;
    feed.error = '';
    const path = `/seerr/discover/${feed.id}`;
    try {
      const read = () =>
        navigationCache.read(
          'discover',
          path,
          () => api<Results>(path),
          (result) => {
            if (active) {
              feed.items = result.results.filter(
                (item) => item.mediaType !== 'person',
              );
              feed.loading = false;
            }
          },
        );
      await (background ? inBackground(read) : read());
    } catch (caught) {
      forgetSeerrStatus();
      if (active) feed.error = String(caught);
    } finally {
      if (active) feed.loading = false;
    }
  }
  $effect(() => {
    if (!feedsReady) return;
    for (const feed of visibleFeeds)
      if (feed.near && !feed.requested) untrack(() => void loadFeed(feed));
  });
  function nearScreen(node: HTMLElement, feed: Feed) {
    const observer = new IntersectionObserver(
      (entries) => (feed.near = entries.some((entry) => entry.isIntersecting)),
      { root: scrollWorkspace(), rootMargin: '600px 0px' },
    );
    observer.observe(node);
    return {
      destroy() {
        observer.disconnect();
        feed.near = false;
      },
    };
  }
  function retryFeed(feed: Feed) {
    navigationCache.invalidate('discover');
    feed.requested = false;
    void loadFeed(feed);
  }
  async function search(nextPage = 1) {
    const term = query.trim(),
      version = ++searchGeneration;
    if (term.length < 2) {
      results = [];
      searching = false;
      return;
    }
    searching = true;
    error = '';
    try {
      const path = `/seerr/search?q=${encodeURIComponent(term)}&page=${nextPage}`;
      await navigationCache.read(
        'discover',
        path,
        () => api<Results>(path),
        (result) => {
          if (!active || version !== searchGeneration) return;
          results =
            nextPage === 1
              ? result.results
              : [
                  ...results,
                  ...result.results.filter(
                    (item) =>
                      !results.some(
                        (old) =>
                          old.id === item.id &&
                          old.mediaType === item.mediaType,
                      ),
                  ),
                ];
          page = result.page;
          pages = result.totalPages;
          total = result.totalResults;
        },
      );
    } catch (caught) {
      if (version === searchGeneration) error = String(caught);
    } finally {
      if (version === searchGeneration) searching = false;
    }
  }
  function changed(publish = true) {
    clearTimeout(searchTimer);
    searchGeneration++;
    results = [];
    page = 1;
    pages = 1;
    if (tab === 'requests') tab = 'discover';
    searching = query.trim().length >= 2;
    if (publish) searchChanged(query);
    searchTimer = setTimeout(() => void search(), 300);
  }
  // `reread` asks Seerr for the titles again, for a reader's own refresh.
  async function loadRequests(nextPage = 1, reread = false) {
    requestedPage = nextPage;
    const version = ++requestGeneration;
    requestLoading = true;
    error = '';
    try {
      const result = await api<{
        results: MediaRequest[];
        pageInfo: { pages: number };
      }>(`/seerr/requests?page=${nextPage}`);
      const details = await Promise.allSettled(
        result.results.map((r) =>
          seerrTitle(r.type || r.media.mediaType, r.media.tmdbId, reread),
        ),
      );
      if (!active || version !== requestGeneration) return;
      requests = result.results.map((r, i) => ({
        ...r,
        detail:
          details[i].status === 'fulfilled' ? details[i].value : undefined,
      }));
      requestPages = result.pageInfo.pages;
      requestPage = nextPage;
      requestLoading = false;
      await tick();
      if (
        !active ||
        version !== requestGeneration ||
        tab !== 'requests' ||
        !configured ||
        selected
      )
        return;
      for (const entry of $attention.filter(
        (item) => item.target === 'requests',
      )) {
        if (
          result.results.some(
            (request) =>
              request.attention?.id === entry.id &&
              request.attention.revision === entry.revision,
          )
        ) {
          acknowledged.push(`${entry.id}:${entry.revision}`);
          void acknowledgeAttention(entry);
        }
      }
    } catch (caught) {
      if (version === requestGeneration) error = String(caught);
    } finally {
      if (version === requestGeneration) requestLoading = false;
    }
  }
  async function requestAction(id: number, action: string) {
    actionBusy = id;
    error = '';
    try {
      await api(`/seerr/requests/${id}/${action}`, 'POST');
      await loadRequests(requestPage);
    } catch (caught) {
      error = String(caught);
    } finally {
      actionBusy = null;
    }
  }
  function changeTab(value: string) {
    if ((value === 'requests') !== showRequests)
      requestsChanged(value === 'requests');
    if (value === 'requests' && tab === value) void loadRequests(1, true);
    tab = value;
    if (value === 'requests') {
      query = '';
      clearTimeout(searchTimer);
      searchGeneration++;
      requestPage = 1;
      requestedPage = 1;
    }
  }
  onMount(() => {
    void load();
  });
  onDestroy(() => {
    active = false;
    searchGeneration++;
    requestGeneration++;
    clearTimeout(searchTimer);
  });
</script>

{#if selected}
  {#key `${selected.kind}:${selected.id}`}<TitlePage
      {user}
      kind={selected.kind === 'tv' ? 'show' : 'movie'}
      tmdb={selected.id}
      {revision}
      {back}
      backLabel="Back to discover"
      {play}
      openTitle={openTarget}
    />{/key}
{:else}
  <div class="w-full min-w-0">
    <form
      class="mb-6 flex items-center gap-3 border border-line bg-surface px-4 focus-within:border-accent"
      data-sidebar-resize="xy-width"
      data-sidebar-group
      onsubmit={(event) => {
        event.preventDefault();
        clearTimeout(searchTimer);
        void search();
      }}
      role="search"
    >
      <Search size={20} class="shrink-0 text-muted" /><input
        class={`${formControlClass} min-w-0 flex-1 border-0 bg-transparent py-4 text-sm shadow-none outline-none focus:ring-0`}
        aria-label="Search movies and series"
        placeholder="Search movies and series…"
        bind:value={query}
        oninput={() => changed()}
        maxlength="200"
      />
      {#if searching}<LoaderCircle
          size={18}
          class="animate-spin text-muted"
        />{:else if query}<Button
          variant="ghost"
          size="sm"
          aria-label="Clear search"
          onclick={() => {
            query = '';
            changed();
          }}><X size={17} /></Button
        >{/if}
    </form>
    <nav
      class="mb-7 flex gap-1 overflow-x-auto border-b border-line"
      data-sidebar-resize="xy-width"
      data-sidebar-group
      aria-label="Discover navigation"
    >
      {#each [{ id: 'discover', label: 'Discover' }, { id: 'movies', label: 'Movies' }, { id: 'tv', label: 'Series' }, { id: 'requests', label: user.role === 'admin' ? 'Requests' : 'My requests' }] as item (item.id)}<button
          class={[
            'shrink-0 border-b-2 px-4 pt-2 pb-3 text-xs font-semibold transition-colors hover:text-foreground',
            tab === item.id
              ? 'border-accent text-accent'
              : 'border-transparent text-muted',
          ]}
          aria-current={tab === item.id ? 'page' : undefined}
          onclick={() => changeTab(item.id)}
          >{item.label}{#if item.id === 'requests'}<AttentionDot
              items={$attention.filter((entry) => entry.target === 'requests')}
              class="ml-2"
            />{/if}</button
        >{/each}
    </nav>
    {#if error}<Notice variant="error" role="alert"
        >{error}<Button
          variant="ghost"
          size="sm"
          onclick={() =>
            void (tab === 'requests'
              ? loadRequests(requestPage, true)
              : query.trim()
                ? search()
                : load())}>Try again</Button
        ></Notice
      >{/if}
    {#if !configured}<section
        class="grid min-h-85 place-content-center justify-items-center gap-4 text-center"
      >
        <Search size={35} class="text-muted" />
        <h2 class="text-lg font-semibold">
          Connect Seerr to discover and request media
        </h2>
        <p class="max-w-110 text-sm leading-6 text-muted">
          {user.role === 'admin'
            ? 'Install Seerr in Media services to get started.'
            : 'Your administrator needs to connect Seerr.'}
        </p>
        {#if user.role === 'admin'}<Button size="form" onclick={settings}
            >Open Media services <ArrowRight size={15} /></Button
          >{/if}
      </section>
    {:else if tab === 'requests'}
      <div class="mb-4 flex items-center justify-between">
        <h2 class="text-base font-semibold">
          {user.role === 'admin' ? 'Media requests' : 'My requests'}
        </h2>
        <Button
          variant="ghost"
          size="sm"
          disabled={requestLoading}
          onclick={() => void loadRequests(requestPage, true)}
          ><RefreshCw size={14} /> Refresh</Button
        >
      </div>
      {#if requestLoading && !requests.length}<ContentSkeleton
          label="Loading requests"
        />{:else if !requests.length}<p
          class="py-12 text-center text-sm text-muted"
        >
          No requests yet. Find a movie or series to get started.
        </p>{:else}<div class="grid gap-3">
          {#each requests as request (request.id)}<article
              class="flex items-center gap-4 border border-line bg-surface p-4 compact:flex-wrap"
            >
              <button
                class="flex min-w-0 flex-1 items-center gap-4 text-left"
                onclick={() =>
                  openTarget({ kind: request.type, id: request.media.tmdbId })}
              >
                {#if artwork(request.detail?.posterPath)}<img
                    class="aspect-2/3 w-13 shrink-0 object-cover"
                    src={artwork(request.detail?.posterPath, 'w185')}
                    srcset={posterSet(request.detail?.posterPath)}
                    sizes="3.25rem"
                    alt=""
                  />{/if}<span
                  ><strong class="block text-sm"
                    >{request.detail
                      ? title(request.detail)
                      : `${request.type === 'movie' ? 'Movie' : 'Series'} #${request.media.tmdbId}`}</strong
                  ><span class="mt-1 block text-xs text-muted"
                    >{request.media.status === 5 || request.status === 5
                      ? 'Available'
                      : (
                          {
                            1: 'Pending approval',
                            2: 'Requested',
                            3: 'Declined',
                            4: 'Failed',
                            5: 'Available',
                          } as Record<number, string>
                        )[request.status]}{request.seasons?.length
                      ? ` · Seasons ${request.seasons.map((s) => s.seasonNumber).join(', ')}`
                      : ''}</span
                  >{#if user.role === 'admin'}<span
                      class="mt-1 block text-[10px] text-muted"
                      >{request.requestedBy?.displayName ||
                        request.requestedBy?.username}</span
                    >{/if}</span
                >
              </button>
              <div class="flex flex-wrap items-center gap-2">
                {#if request.status === 1 && user.role === 'admin'}<Button
                    size="sm"
                    disabled={actionBusy !== null}
                    onclick={() => void requestAction(request.id, 'approve')}
                    >Approve</Button
                  ><Button
                    variant="secondary"
                    size="sm"
                    disabled={actionBusy !== null}
                    onclick={() => void requestAction(request.id, 'decline')}
                    >Decline</Button
                  >{/if}
                {#if request.status === 1 || (request.status === 4 && user.role === 'admin')}<DropdownMenu.Root
                    open={requestMenu === request.id}
                    onOpenChange={(open) =>
                      (requestMenu = open ? request.id : null)}
                  >
                    <DropdownMenu.Trigger disabled={actionBusy !== null}>
                      {#snippet child({ props })}<Button
                          {...props}
                          variant="ghost"
                          size="compact-icon"
                          aria-label={`More actions for ${request.detail ? title(request.detail) : 'this request'}`}
                          ><Ellipsis size={16} /></Button
                        >{/snippet}
                    </DropdownMenu.Trigger>
                    <DropdownMenu.Portal>
                      {#if requestMenu === request.id}<MenuBackdrop
                          close={() => (requestMenu = null)}
                        />{/if}
                      <DropdownMenu.Content
                        align="end"
                        sideOffset={4}
                        preventScroll={false}
                        onInteractOutside={ignoreBackdrop}
                        class="z-50 grid min-w-44 border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
                      >
                        {#if request.status === 4 && user.role === 'admin'}<DropdownMenu.Item
                            class={menuItemClass}
                            onSelect={() =>
                              void requestAction(request.id, 'retry')}
                            ><RotateCcw />Retry request</DropdownMenu.Item
                          >{/if}
                        {#if request.status === 1}<DropdownMenu.Item
                            class={menuItemClass}
                            onSelect={() =>
                              void requestAction(request.id, 'cancel')}
                            ><Ban />Cancel request</DropdownMenu.Item
                          >{/if}
                      </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                  </DropdownMenu.Root>{/if}
              </div>
            </article>{/each}
        </div>{/if}
      {#if requestPages > 1}<div class="mt-5 flex justify-center gap-4">
          <Button
            variant="secondary"
            size="sm"
            disabled={requestPage <= 1 || requestLoading}
            onclick={() => void loadRequests(requestPage - 1)}>Previous</Button
          ><span class="self-center text-xs text-muted"
            >{requestPage} / {requestPages}</span
          ><Button
            variant="secondary"
            size="sm"
            disabled={requestPage >= requestPages || requestLoading}
            onclick={() => void loadRequests(requestPage + 1)}>Next</Button
          >
        </div>{/if}
    {:else if query.trim()}
      {#if query.trim().length < 2}<p class="py-8 text-sm text-muted">
          Enter at least two characters.
        </p>{:else if searching && !results.length}<MediaSkeleton
          label="Loading search results"
          layout="posters"
        />{:else if !searching && !filtered.length}<p
          class="py-8 text-sm text-muted"
        >
          No results for “{query}”.
        </p>{:else}<p class="mb-5 text-xs text-muted">
          {searching && !results.length
            ? 'Searching…'
            : `Results for “${query}”`}{!searching &&
          tab === 'discover' &&
          total
            ? ` · ${total}`
            : ''}
        </p>
        <div
          class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-3 compact:gap-3"
        >
          {#each filtered as item (`${item.mediaType}:${item.id}`)}<PosterCard
              {item}
              {open}
              sizes="(max-width: 720px) 34vw, 19rem"
            />{/each}
        </div>{/if}
      {#if page < pages}<div class="mt-7 flex justify-center">
          <Button
            variant="secondary"
            size="form"
            disabled={searching}
            onclick={() => void search(page + 1)}
            >{searching ? 'Loading…' : 'Load more'}</Button
          >
        </div>{/if}
    {:else}
      {#if featured}<button
          class="relative mb-9 flex min-h-75 w-full items-end overflow-hidden border border-line bg-surface p-7 text-left compact:min-h-60 compact:p-5"
          data-sidebar-panel
          onclick={() => open(featured)}
          ><img
            class="absolute inset-0 size-full object-cover object-center opacity-45"
            src={artwork(featured.backdropPath, 'w1280')}
            srcset={backdropSet(featured.backdropPath)}
            sizes="100vw"
            fetchpriority="high"
            alt=""
          /><span
            class="absolute inset-0 bg-linear-to-r from-background via-background/65 to-transparent"
          ></span><span class="relative max-w-160"
            ><span
              class="mb-3 block text-[10px] font-semibold tracking-[0.16em] text-accent uppercase"
              >Trending now · {year(featured)}</span
            ><strong
              class="block text-3xl leading-tight font-semibold tracking-tight compact:text-2xl"
              >{title(featured)}</strong
            ><span class="mt-3 line-clamp-2 block text-xs leading-6 text-muted"
              >{featured.overview}</span
            ><span
              class="mt-5 inline-flex items-center gap-2 text-xs font-semibold text-accent"
              >View details <ArrowRight size={15} /></span
            ></span
          ></button
        >{/if}
      <div class="grid gap-7">
        {#each visibleFeeds as feed (feed.id)}{#if feed.items.length}<MediaRow
              label={feed.label}
              items={feed.items}
              {open}
            />{:else if feed.loading}<div use:nearScreen={feed}>
              <MediaSkeleton
                label={`Loading ${feed.label}`}
                heading={feed.label}
                layout="poster-row"
                count={6}
              />
            </div>{:else if feed.error}<Notice variant="error"
              ><strong>{feed.label}</strong>
              <p>{feed.error}</p>
              <Button variant="ghost" size="sm" onclick={() => retryFeed(feed)}
                >Try again</Button
              ></Notice
            >{/if}{/each}
      </div>
    {/if}
    <p class="mt-8 text-[10px] text-muted">Metadata from TMDB.</p>
  </div>
{/if}
