<script lang="ts">
  import { Popover } from 'bits-ui';
  import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    Layers3,
    ListFilter,
    ListPlus,
    LoaderCircle,
    Play,
    PanelRightClose,
    PanelRightOpen,
    RefreshCw,
    RotateCw,
    Search,
  } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { relativeTime } from '../../utils';
  import {
    validWatchStates as allWatchStates,
    type YoutubeFeedPreferences,
  } from '../../youtube-feed-preferences';
  import type {
    YoutubeChannelOption,
    YoutubeVideoCounts,
    SyncStatus,
    YoutubeSortField,
    YoutubeSortDirection,
    YoutubeWatchState,
    YoutubeGrouping,
    YoutubeDurationFilter,
    YoutubePublishedFilter,
    YoutubeDownloadFilter,
  } from '../../types';
  let {
    filters = $bindable(),
    watchlistSidebarOpen = $bindable(),
    onFullRefresh,
    channels,
    counts,
    syncStatus,
    directVideoId,
    directLaunchState,
    directWatchlistBusy,
    directInWatchlist,
    directWatchlistName,
    addDirectVideoToWatchlist,
    hasActiveFilters,
    submitSearch,
    clearFilters,
    setGrouping,
    refresh,
  }: {
    filters: YoutubeFeedPreferences;
    watchlistSidebarOpen: boolean;
    onFullRefresh: () => void;
    channels: YoutubeChannelOption[];
    counts: YoutubeVideoCounts;
    syncStatus: SyncStatus;
    directVideoId: string | null;
    directLaunchState: string | null;
    directWatchlistBusy: boolean;
    directInWatchlist: boolean;
    directWatchlistName: string | null;
    addDirectVideoToWatchlist: () => Promise<void>;
    hasActiveFilters: boolean;
    submitSearch: (event: SubmitEvent) => Promise<void>;
    clearFilters: () => void;
    setGrouping: (group: YoutubeGrouping) => Promise<void>;
    refresh: (mode: 'normal' | 'full') => Promise<void>;
  } = $props();
  let openMenu = $state<'sort' | 'filter' | 'group' | null>(null);
  function setMenuOpen(menu: 'sort' | 'filter' | 'group', open: boolean) {
    if (open) openMenu = menu;
    else if (openMenu === menu) openMenu = null;
  }

  function searchKeydown(event: KeyboardEvent) {
    if (
      event.key !== 'Enter' ||
      !event.ctrlKey ||
      event.altKey ||
      event.shiftKey ||
      event.isComposing ||
      !directVideoId
    )
      return;
    event.preventDefault();
    if (!event.repeat) void addDirectVideoToWatchlist();
  }
  function setSort(field: YoutubeSortField, direction: YoutubeSortDirection) {
    filters.sortField = field;
    filters.sortDirection = direction;
  }
  function toggleWatchState(state: YoutubeWatchState) {
    if (filters.watchStates.includes(state)) {
      if (filters.watchStates.length > 1)
        filters.watchStates = filters.watchStates.filter(
          (value) => value !== state,
        );
    } else filters.watchStates = [...filters.watchStates, state];
  }
</script>

<div class="relative z-30 mr-3 flex shrink-0 items-center gap-1.5 pb-2">
  <span
    data-sidebar-resize="x"
    aria-hidden="true"
    class="pointer-events-none absolute right-0 bottom-0 left-0 h-px bg-(--line)"
  ></span>
  <form
    data-sidebar-resize="x"
    class={`relative flex h-9 min-w-0 flex-1 items-center border bg-(--surface) focus-within:border-(--line-strong) ${directVideoId ? 'border-(--line-strong)' : 'border-(--line)'}`}
    onsubmit={submitSearch}
  >
    {#if directLaunchState === 'launching'}
      <LoaderCircle
        class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 animate-spin text-(--accent)"
      />
    {:else if directVideoId}
      <Play
        class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-(--accent)"
      />
    {:else}
      <Search
        class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-(--muted)"
      />
    {/if}
    <input
      bind:value={filters.searchText}
      onkeydown={searchKeydown}
      type="search"
      maxlength="200"
      placeholder="Search videos or channels, or paste a YouTube URL or ID"
      aria-label={directVideoId
        ? `YouTube video ${directVideoId}`
        : 'Search YouTube feed'}
      aria-describedby="youtube-search-mode"
      class="h-full min-w-0 flex-1 bg-transparent pr-3 pl-9 text-sm outline-none placeholder:text-(--muted)"
    />
    {#if directVideoId}
      <div
        class="mr-2 flex shrink-0 items-center gap-2 text-[0.625rem] text-(--accent)"
      >
        <button
          type="submit"
          class="flex items-center gap-1 whitespace-nowrap hover:underline disabled:opacity-60"
          disabled={directLaunchState !== 'ready'}
          title="Play video (Enter)"
          aria-keyshortcuts="Enter"
        >
          <Play class="size-3" />
          {directLaunchState === 'launching'
            ? 'Launching…'
            : directLaunchState === 'active'
              ? 'Already playing'
              : 'Play'}
          {#if directLaunchState === 'ready'}<kbd
              class="rounded border border-(--line-strong) px-1 font-sans"
              >Enter</kbd
            >{/if}
        </button>
        {#if directWatchlistName}
          <button
            type="button"
            class="flex items-center gap-1 border-l border-(--line-strong) pl-2 whitespace-nowrap hover:underline disabled:opacity-60"
            disabled={directWatchlistBusy || directInWatchlist}
            title={directInWatchlist
              ? `Already in ${directWatchlistName}`
              : `Add to ${directWatchlistName} (Ctrl+Enter)`}
            aria-label={directInWatchlist
              ? `Already in ${directWatchlistName}`
              : `Add to ${directWatchlistName}`}
            aria-keyshortcuts="Control+Enter"
            onclick={addDirectVideoToWatchlist}
          >
            {#if directWatchlistBusy}<LoaderCircle
                class="size-3 animate-spin"
              />{:else}<ListPlus class="size-3" />{/if}
            {directWatchlistBusy
              ? 'Adding…'
              : directInWatchlist
                ? 'Added'
                : directWatchlistName}
            <kbd class="rounded border border-(--line-strong) px-1 font-sans"
              >Ctrl+Enter</kbd
            >
          </button>
        {/if}
      </div>
    {/if}
    <span id="youtube-search-mode" class="sr-only" aria-live="polite">
      {directLaunchState === 'launching'
        ? 'Launching'
        : directLaunchState === 'active'
          ? 'Already playing'
          : directVideoId
            ? 'Enter to play'
            : 'Typing filters the YouTube feed'}
      {#if directVideoId && directWatchlistName}
        {directWatchlistBusy
          ? 'Adding to watchlist.'
          : directInWatchlist
            ? `Already in ${directWatchlistName}.`
            : `Ctrl+Enter to add to ${directWatchlistName}.`}
      {/if}
    </span>
  </form>

  <div class="relative z-30">
    <Popover.Root
      open={openMenu === 'sort'}
      onOpenChange={(open) => setMenuOpen('sort', open)}
    >
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            size="icon"
            variant={openMenu === 'sort' ? 'secondary' : 'ghost'}
            aria-label="Sort feed"
            title="Sort feed"
            aria-expanded={openMenu === 'sort'}
            ><ArrowUpDown class="size-4" /></Button
          >
        {/snippet}
      </Popover.Trigger>
      <Popover.ContentStatic
        role="dialog"
        aria-label="Sort feed"
        class="panel absolute top-11 right-0 w-64 border border-(--line-strong) bg-(--surface-strong) p-3 shadow-xl"
      >
        <p class="eyebrow mb-2">SORT BY</p>
        {#each [['date', 'Date'], ['channel', 'Channel'], ['duration', 'Duration']] as item (item[0])}
          <div
            class="flex items-center gap-1 border-t border-(--line) py-1.5 first:border-0"
          >
            <span class="flex-1 text-sm">{item[1]}</span>
            {#each [['asc', ArrowUp], ['desc', ArrowDown]] as direction (direction[0])}
              <button
                type="button"
                class={`grid size-8 place-items-center border ${filters.sortField === item[0] && filters.sortDirection === direction[0] ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted) hover:text-(--foreground)'}`}
                aria-label={`${item[1]} ${direction[0] === 'asc' ? 'ascending' : 'descending'}`}
                aria-pressed={filters.sortField === item[0] &&
                  filters.sortDirection === direction[0]}
                onclick={() =>
                  setSort(
                    item[0] as YoutubeSortField,
                    direction[0] as YoutubeSortDirection,
                  )}
                >{#if direction[0] === 'asc'}<ArrowUp
                    class="size-3.5"
                  />{:else}<ArrowDown class="size-3.5" />{/if}</button
              >
            {/each}
          </div>
        {/each}
      </Popover.ContentStatic>
    </Popover.Root>
  </div>

  <div class="relative z-30">
    <Popover.Root
      open={openMenu === 'filter'}
      onOpenChange={(open) => setMenuOpen('filter', open)}
    >
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            size="icon"
            variant={openMenu === 'filter' || hasActiveFilters
              ? 'secondary'
              : 'ghost'}
            aria-label="Filter feed"
            title="Filter feed"
            aria-expanded={openMenu === 'filter'}
            ><ListFilter class="size-4" /></Button
          >
        {/snippet}
      </Popover.Trigger>
      <Popover.ContentStatic
        role="dialog"
        aria-label="Filter feed"
        class="panel absolute top-11 right-0 max-h-[calc(100dvh-7rem)] w-80 overflow-y-auto border border-(--line-strong) bg-(--surface-strong) p-3 shadow-xl"
      >
        <div class="mb-2 flex items-center justify-between">
          <p class="eyebrow">FILTER</p>
          {#if hasActiveFilters}<button
              type="button"
              class="text-xs text-(--accent) hover:underline"
              onclick={clearFilters}>Clear</button
            >{/if}
        </div>
        <p class="mt-3 mb-1.5 text-xs text-(--muted)">Watch state</p>
        <div class="flex flex-wrap gap-1.5">
          <button
            type="button"
            class={`h-8 border px-2.5 text-xs ${filters.watchStates.length === 3 ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
            aria-pressed={filters.watchStates.length === 3}
            onclick={() => (filters.watchStates = [...allWatchStates])}
            >All</button
          >
          {#each [['unwatched', 'Unwatched', counts.unwatched], ['in_progress', 'In progress', counts.inProgress], ['watched', 'Watched', counts.watched]] as item (item[0])}
            <button
              type="button"
              class={`h-8 border px-2.5 text-xs ${filters.watchStates.includes(item[0] as YoutubeWatchState) ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
              aria-pressed={filters.watchStates.includes(
                item[0] as YoutubeWatchState,
              )}
              onclick={() => toggleWatchState(item[0] as YoutubeWatchState)}
              >{item[1]} <span class="opacity-60">{item[2]}</span></button
            >
          {/each}
        </div>
        <p class="mt-4 mb-1.5 text-xs text-(--muted)">Video type</p>
        <div class="flex flex-wrap gap-1.5">
          <button
            type="button"
            class={`h-8 border px-2.5 text-xs ${filters.showShorts ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
            aria-pressed={filters.showShorts}
            onclick={() => (filters.showShorts = !filters.showShorts)}
            >Shorts <span class="opacity-60">{counts.shorts}</span></button
          >
          <button
            type="button"
            class={`h-8 border px-2.5 text-xs ${filters.showLive ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
            aria-pressed={filters.showLive}
            onclick={() => (filters.showLive = !filters.showLive)}
            >Live <span class="opacity-60">{counts.live}</span></button
          >
          <button
            type="button"
            class={`h-8 border px-2.5 text-xs ${filters.showLiveReplays ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
            aria-pressed={filters.showLiveReplays}
            onclick={() => (filters.showLiveReplays = !filters.showLiveReplays)}
            >Replays <span class="opacity-60">{counts.liveReplays}</span
            ></button
          >
          <button
            type="button"
            class={`h-8 border px-2.5 text-xs ${filters.showUpcoming ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
            aria-pressed={filters.showUpcoming}
            onclick={() => (filters.showUpcoming = !filters.showUpcoming)}
            >Upcoming <span class="opacity-60">{counts.upcoming}</span></button
          >
        </div>
        <label
          class="mt-4 block text-xs text-(--muted)"
          for="youtube-channel-filter">Channel</label
        >
        <select
          id="youtube-channel-filter"
          bind:value={filters.channelId}
          class="mt-1.5 h-9 w-full border border-(--line) bg-(--surface) px-2 text-sm"
        >
          <option value="">All channels</option>
          {#each channels as channel (channel.channelId)}
            <option value={channel.channelId}>{channel.title}</option>
          {/each}
        </select>
        <p class="mt-4 mb-1.5 text-xs text-(--muted)">Length</p>
        <div class="flex flex-wrap gap-1.5">
          {#each [['any', 'Any'], ['under_10', 'Under 10m'], ['10_plus', '10m+'], ['30_plus', '30m+']] as item (item[0])}
            <button
              type="button"
              class={`h-8 border px-2.5 text-xs ${filters.durationFilter === item[0] ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
              aria-pressed={filters.durationFilter === item[0]}
              onclick={() =>
                (filters.durationFilter = item[0] as YoutubeDurationFilter)}
              >{item[1]}</button
            >
          {/each}
        </div>
        <p class="mt-4 mb-1.5 text-xs text-(--muted)">Published</p>
        <div class="flex flex-wrap gap-1.5">
          {#each [['any', 'Any'], ['7_days', 'Past 7 days'], ['30_days', 'Past 30 days']] as item (item[0])}
            <button
              type="button"
              class={`h-8 border px-2.5 text-xs ${filters.publishedFilter === item[0] ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
              aria-pressed={filters.publishedFilter === item[0]}
              onclick={() =>
                (filters.publishedFilter = item[0] as YoutubePublishedFilter)}
              >{item[1]}</button
            >
          {/each}
        </div>
        <p class="mt-4 mb-1.5 text-xs text-(--muted)">Storage</p>
        <div class="flex flex-wrap gap-1.5">
          {#each [['all', 'All videos'], ['downloaded', 'Downloaded']] as item (item[0])}
            <button
              type="button"
              class={`h-8 border px-2.5 text-xs ${filters.downloadFilter === item[0] ? 'border-(--line-strong) bg-(--accent-soft) text-(--accent)' : 'border-(--line) text-(--muted)'}`}
              aria-pressed={filters.downloadFilter === item[0]}
              onclick={() =>
                (filters.downloadFilter = item[0] as YoutubeDownloadFilter)}
              >{item[1]}</button
            >
          {/each}
        </div>
      </Popover.ContentStatic>
    </Popover.Root>
  </div>

  <div class="relative z-30">
    <Popover.Root
      open={openMenu === 'group'}
      onOpenChange={(open) => setMenuOpen('group', open)}
    >
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            size="icon"
            variant={openMenu === 'group' ? 'secondary' : 'ghost'}
            aria-label="Group feed"
            title="Group feed"
            aria-expanded={openMenu === 'group'}
            ><Layers3 class="size-4" /></Button
          >
        {/snippet}
      </Popover.Trigger>
      <Popover.ContentStatic
        role="dialog"
        aria-label="Group feed"
        class="panel absolute top-11 right-0 w-72 border border-(--line-strong) bg-(--surface-strong) p-2 shadow-xl"
      >
        <p class="eyebrow px-2 py-1">GROUP BY DATE</p>
        {#each [['smart', 'Smart', 'Days, then weeks, then months'], ['day', 'Every day', 'One dated section per day'], ['week', 'Every week', 'Monday through Sunday'], ['month', 'Every month', 'One section per month'], ['none', 'No grouping', 'One sorted feed']] as item (item[0])}
          <button
            type="button"
            class={`mt-1 w-full border px-2.5 py-2 text-left ${filters.grouping === item[0] ? 'border-(--line-strong) bg-(--accent-soft)' : 'border-transparent hover:border-(--line)'}`}
            aria-pressed={filters.grouping === item[0]}
            onclick={() => setGrouping(item[0] as YoutubeGrouping)}
          >
            <span class="block text-sm">{item[1]}</span>
            <span class="block text-xs text-(--muted)">{item[2]}</span>
          </button>
        {/each}
      </Popover.ContentStatic>
    </Popover.Root>
  </div>

  <span class="mx-0.5 h-5 w-px bg-(--line)" aria-hidden="true"></span>
  <Button
    size="icon"
    variant="ghost"
    disabled={syncStatus.isRefreshing}
    aria-label="Refresh YouTube feed"
    title={syncStatus.isRefreshing
      ? `${syncStatus.phase} · ${syncStatus.completed}/${syncStatus.total ?? '—'}`
      : `Refresh · updated ${relativeTime(syncStatus.lastSuccessAt)}`}
    onclick={() => refresh('normal')}
    ><RefreshCw
      class={`size-4 ${syncStatus.isRefreshing ? 'animate-spin' : ''}`}
    /></Button
  >
  <Button
    size="icon"
    variant="ghost"
    disabled={syncStatus.isRefreshing}
    aria-label="Full YouTube refresh"
    title="Full YouTube refresh"
    onclick={onFullRefresh}><RotateCw class="size-4" /></Button
  >
  <span class="mx-0.5 h-5 w-px bg-(--line)" aria-hidden="true"></span>
  <Button
    size="icon"
    variant={watchlistSidebarOpen ? 'secondary' : 'ghost'}
    aria-label={watchlistSidebarOpen ? 'Close watchlists' : 'Open watchlists'}
    title={watchlistSidebarOpen ? 'Close watchlists' : 'Open watchlists'}
    aria-pressed={watchlistSidebarOpen}
    onclick={() => (watchlistSidebarOpen = !watchlistSidebarOpen)}
    >{#if watchlistSidebarOpen}<PanelRightClose
        class="size-4"
      />{:else}<PanelRightOpen class="size-4" />{/if}</Button
  >
</div>

<style>
  input:focus-visible {
    outline: none;
  }

  form:has(input:focus-visible) {
    border-color: var(--accent);
  }
</style>
