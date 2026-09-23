<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import {
    Search,
    X,
    LoaderCircle,
    ArrowRight,
    RefreshCw,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  import MediaRow from './MediaRow.svelte';
  import PosterCard from './PosterCard.svelte';
  import MediaDetails from './MediaDetails.svelte';
  import {
    artwork,
    title,
    kind,
    year,
    type Media,
    type MediaKind,
    type Results,
    type MediaRequest,
  } from './types';
  let { user, settings } = $props<{ user: User; settings: () => void }>();
  type Feed = { id: string; label: string; items: Media[]; error: string };
  let feeds = $state<Feed[]>([
    { id: 'trending', label: 'Trending', items: [], error: '' },
    { id: 'movies', label: 'Popular movies', items: [], error: '' },
    { id: 'tv', label: 'Popular series', items: [], error: '' },
    { id: 'upcoming', label: 'Upcoming movies', items: [], error: '' },
    { id: 'upcoming-tv', label: 'Upcoming series', items: [], error: '' },
  ]);
  let tab = $state('discover'),
    query = $state(''),
    results = $state<Media[]>([]),
    page = $state(1),
    pages = $state(1),
    total = $state(0),
    searching = $state(false),
    loading = $state(true),
    configured = $state(true),
    error = $state('');
  let selected = $state<{ kind: MediaKind; id: number } | null>(null);
  let requests = $state<(MediaRequest & { detail?: Media })[]>([]),
    requestPages = $state(1),
    requestPage = $state(1),
    requestLoading = $state(false),
    actionBusy = $state<number | null>(null);
  let searchTimer: ReturnType<typeof setTimeout>,
    searchGeneration = 0,
    requestGeneration = 0,
    active = true;
  let browseScroll = 0;
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
  function route() {
    const wasDetails = selected !== null;
    const match = /^#discover\/(movie|tv)\/(\d+)$/.exec(location.hash);
    selected = match
      ? { kind: match[1] as MediaKind, id: Number(match[2]) }
      : null;
    if (wasDetails && !selected)
      void tick().then(() => {
        if (active && !selected)
          scrollWorkspace()?.scrollTo({ top: browseScroll });
      });
  }
  function open(item: Media) {
    if (!selected) browseScroll = scrollWorkspace()?.scrollTop ?? 0;
    history.pushState(
      { ...history.state, seerr: true },
      '',
      `${location.pathname}${location.search}#discover/${kind(item)}/${item.id}`,
    );
    route();
    scrollWorkspace()?.scrollTo({ top: 0 });
  }
  function back() {
    if (history.state?.seerr) history.back();
    else {
      history.replaceState(
        history.state,
        '',
        `${location.pathname}${location.search}`,
      );
      route();
    }
  }
  async function load() {
    loading = true;
    error = '';
    try {
      const state = await api<{ configured: boolean; ready: boolean }>(
        '/seerr/status',
      );
      if (!active) return;
      configured = state.configured;
      if (!configured) return;
      if (!state.ready) {
        error = 'Seerr is still completing setup. Try again shortly.';
        return;
      }
      await Promise.all(
        feeds.map(async (feed) => {
          try {
            const result = await api<Results>(`/seerr/discover/${feed.id}`);
            if (active) {
              feed.items = result.results.filter(
                (item) => item.mediaType !== 'person',
              );
              feed.error = '';
            }
          } catch (caught) {
            if (active) feed.error = String(caught);
          }
        }),
      );
    } catch (caught) {
      if (active) error = String(caught);
    } finally {
      if (active) loading = false;
    }
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
      const result = await api<Results>(
        `/seerr/search?q=${encodeURIComponent(term)}&page=${nextPage}`,
      );
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
                      old.id === item.id && old.mediaType === item.mediaType,
                  ),
              ),
            ];
      page = result.page;
      pages = result.totalPages;
      total = result.totalResults;
    } catch (caught) {
      if (version === searchGeneration) error = String(caught);
    } finally {
      if (version === searchGeneration) searching = false;
    }
  }
  function changed() {
    clearTimeout(searchTimer);
    searchGeneration++;
    results = [];
    page = 1;
    pages = 1;
    if (tab === 'requests') tab = 'discover';
    searching = query.trim().length >= 2;
    searchTimer = setTimeout(() => void search(), 300);
  }
  async function loadRequests(nextPage = 1) {
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
          api<Media>(`/seerr/${r.type || r.media.mediaType}/${r.media.tmdbId}`),
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
    tab = value;
    if (value === 'requests') {
      query = '';
      clearTimeout(searchTimer);
      searchGeneration++;
      void loadRequests();
    }
  }
  onMount(() => {
    route();
    window.addEventListener('popstate', route);
    window.addEventListener('hashchange', route);
    void load();
  });
  onDestroy(() => {
    active = false;
    searchGeneration++;
    requestGeneration++;
    clearTimeout(searchTimer);
    window.removeEventListener('popstate', route);
    window.removeEventListener('hashchange', route);
  });
</script>

{#if selected}
  {#key `${selected.kind}:${selected.id}`}<MediaDetails
      mediaKind={selected.kind}
      id={selected.id}
      {back}
      {open}
    />{/key}
{:else}
  <div class="w-full min-w-0">
    <form
      class="mb-6 flex items-center gap-3 border border-line bg-surface px-4 focus-within:border-accent"
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
        oninput={changed}
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
          onclick={() => changeTab(item.id)}>{item.label}</button
        >{/each}
    </nav>
    {#if error}<Notice variant="error" role="alert"
        >{error}<Button
          variant="ghost"
          size="sm"
          onclick={() =>
            void (tab === 'requests'
              ? loadRequests(requestPage)
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
          onclick={() => void loadRequests(requestPage)}
          ><RefreshCw size={14} /> Refresh</Button
        >
      </div>
      {#if requestLoading}<p class="py-8 text-sm text-muted" role="status">
          Loading requests…
        </p>{:else if !requests.length}<p
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
                  open(
                    request.detail
                      ? { ...request.detail, mediaType: request.type }
                      : { id: request.media.tmdbId, mediaType: request.type },
                  )}
              >
                {#if artwork(request.detail?.posterPath)}<img
                    class="aspect-2/3 w-13 shrink-0 object-cover"
                    src={artwork(request.detail?.posterPath, 'w185')}
                    alt=""
                  />{/if}<span
                  ><strong class="block text-sm"
                    >{request.detail
                      ? title(request.detail)
                      : `${request.type === 'movie' ? 'Movie' : 'Series'} #${request.media.tmdbId}`}</strong
                  ><span class="mt-1 block text-xs text-muted"
                    >{request.media.status === 5
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
              <div class="flex flex-wrap gap-2">
                {#if request.status === 1}{#if user.role === 'admin'}<Button
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
                    >{:else}<Button
                      variant="secondary"
                      size="sm"
                      disabled={actionBusy !== null}
                      onclick={() => void requestAction(request.id, 'cancel')}
                      >Cancel</Button
                    >{/if}{:else if request.status === 4 && user.role === 'admin'}<Button
                    size="sm"
                    disabled={actionBusy !== null}
                    onclick={() => void requestAction(request.id, 'retry')}
                    >Retry</Button
                  >{/if}
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
        </p>{:else if !searching && !filtered.length}<p
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
          class="grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-x-5 gap-y-7 compact:grid-cols-2 compact:gap-3"
        >
          {#each filtered as item (`${item.mediaType}:${item.id}`)}<PosterCard
              {item}
              {open}
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
    {:else if loading}<div
        class="grid gap-8"
        aria-label="Loading discovery"
        role="status"
      >
        {#each [1, 2, 3] as row (row)}<div
            class="grid grid-cols-6 gap-4 compact:grid-cols-3"
          >
            {#each [1, 2, 3, 4, 5, 6] as card (card)}<div
                class="aspect-2/3 animate-pulse bg-surface-strong motion-reduce:animate-none"
              ></div>{/each}
          </div>{/each}
      </div>
    {:else}
      {#if featured}<button
          class="relative mb-9 flex min-h-75 w-full items-end overflow-hidden border border-line bg-surface p-7 text-left compact:min-h-60 compact:p-5"
          onclick={() => open(featured)}
          ><img
            class="absolute inset-0 size-full object-cover object-center opacity-45"
            src={artwork(featured.backdropPath, 'w1280')}
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
            />{:else if feed.error}<Notice variant="error"
              ><strong>{feed.label}</strong>
              <p>{feed.error}</p>
              <Button variant="ghost" size="sm" onclick={() => void load()}
                >Try again</Button
              ></Notice
            >{/if}{/each}
      </div>
    {/if}
    <p class="mt-8 text-[10px] text-muted">Metadata from TMDB.</p>
  </div>
{/if}
