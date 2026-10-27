<script lang="ts">
  import { Popover } from 'bits-ui';
  import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    ListFilter,
    Search,
  } from '@lucide/svelte';
  import Button from './ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from './ui/MenuBackdrop.svelte';
  import { eyebrowTextClass, toolbarMenuClass } from './ui/styles';
  import {
    availabilities,
    choose,
    filtering,
    seriesStates,
    sortsFor,
    watchStates,
    type LibraryCounts,
    type LibraryView,
    type SortOrder,
    unfiltered,
  } from './library-view';
  let {
    domain,
    search = $bindable(),
    searchChanged,
    view,
    viewChanged,
    counts,
    genres,
    collections,
    collection = $bindable(),
    collectionChanged,
  }: {
    domain: string;
    search: string;
    searchChanged: () => void;
    /** Movies and Shows sort and filter; other lists only search. */
    view?: LibraryView;
    viewChanged: (view: LibraryView) => void;
    counts?: LibraryCounts;
    genres: string[];
    collections: { id: number; name: string; count: number }[];
    collection: string;
    collectionChanged: () => void;
  } = $props();
  const orders: SortOrder[] = ['asc', 'desc'];
  const shows = $derived(domain === 'Shows');
  const filtered = $derived(
    !!view && (filtering(view, shows) || collection !== ''),
  );
  // A saved genre stays choosable after the library no longer has it.
  const genreOptions = $derived(
    view?.genre && !genres.includes(view.genre)
      ? [view.genre, ...genres]
      : genres,
  );
  let openMenu = $state<'sort' | 'filter' | null>(null);
  function setMenuOpen(menu: 'sort' | 'filter', open: boolean) {
    if (open) openMenu = menu;
    else if (openMenu === menu) openMenu = null;
  }
  function change(next: Partial<LibraryView>) {
    if (view) viewChanged({ ...view, ...next });
  }
  function clear() {
    if (view) viewChanged(unfiltered(view));
    if (collection) {
      collection = '';
      collectionChanged();
    }
  }
  // Options read as chosen only once the list is narrowed; until then "All" is.
  const narrowed = <T,>(chosen: T[], options: [T, string][], option: T) =>
    chosen.length < options.length && chosen.includes(option);
  const chipClass = (active: boolean) =>
    `h-8 border px-2.5 text-xs ${active ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted'}`;
  const selectClass =
    'mt-1.5 h-9 w-full border border-line bg-surface px-2 text-sm';
</script>

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
    placeholder={`Search your ${domain.toLowerCase()}`}
    aria-label={`Search ${domain.toLowerCase()}`}
    class="h-full min-w-0 flex-1 bg-transparent pr-3 pl-9 text-sm outline-none placeholder:text-muted"
  />
</div>
{#if view}
  <div class="relative z-30 compact:static">
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
            aria-label={`Sort ${domain.toLowerCase()}`}
            title={`Sort ${domain.toLowerCase()}`}
            aria-expanded={openMenu === 'sort'}
            ><ArrowUpDown class="size-4" /></Button
          >
        {/snippet}
      </Popover.Trigger>
      {#if openMenu === 'sort'}<MenuBackdrop
          close={() => (openMenu = null)}
        />{/if}
      <Popover.ContentStatic
        role="dialog"
        aria-label={`Sort ${domain.toLowerCase()}`}
        onInteractOutside={ignoreBackdrop}
        class={`absolute top-11 left-0 z-50 w-64 border border-line-strong bg-surface-strong p-3 shadow-xl ${toolbarMenuClass}`}
      >
        <p class={`${eyebrowTextClass} mb-2`}>SORT BY</p>
        {#each sortsFor(shows) as [sort, label] (sort)}
          <div
            class="flex items-center gap-1 border-t border-line py-1.5 first:border-0"
          >
            <span class="flex-1 text-sm">{label}</span>
            {#each orders as order (order)}
              <button
                type="button"
                class={`grid size-8 place-items-center border ${view.sort === sort && view.order === order ? 'border-line-strong bg-accent-soft text-accent' : 'border-line text-muted hover:text-foreground'}`}
                aria-label={`${label} ${order === 'asc' ? 'ascending' : 'descending'}`}
                aria-pressed={view.sort === sort && view.order === order}
                onclick={() => change({ sort, order })}
                >{#if order === 'asc'}<ArrowUp
                    class="size-3.5"
                  />{:else}<ArrowDown class="size-3.5" />{/if}</button
              >
            {/each}
          </div>
        {/each}
      </Popover.ContentStatic>
    </Popover.Root>
  </div>

  <div class="relative z-30 compact:static">
    <Popover.Root
      open={openMenu === 'filter'}
      onOpenChange={(open) => setMenuOpen('filter', open)}
    >
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            size="icon"
            variant={openMenu === 'filter' || filtered ? 'secondary' : 'ghost'}
            aria-label={`Filter ${domain.toLowerCase()}`}
            title={`Filter ${domain.toLowerCase()}`}
            aria-expanded={openMenu === 'filter'}
            ><ListFilter class="size-4" /></Button
          >
        {/snippet}
      </Popover.Trigger>
      {#if openMenu === 'filter'}<MenuBackdrop
          close={() => (openMenu = null)}
        />{/if}
      <Popover.ContentStatic
        role="dialog"
        aria-label={`Filter ${domain.toLowerCase()}`}
        onInteractOutside={ignoreBackdrop}
        class={`absolute top-11 left-0 z-50 max-h-[calc(100dvh-7rem)] w-80 overflow-y-auto border border-line-strong bg-surface-strong p-3 shadow-xl ${toolbarMenuClass}`}
      >
        <div class="mb-2 flex items-center justify-between">
          <p class={eyebrowTextClass}>FILTER</p>
          {#if filtered}<button
              type="button"
              class="text-xs text-accent hover:underline"
              onclick={clear}>Clear</button
            >{/if}
        </div>
        <p class="mt-3 mb-1.5 text-xs text-muted">Watch state</p>
        <div class="flex flex-wrap gap-1.5">
          <button
            type="button"
            class={chipClass(view.watch.length === watchStates.length)}
            aria-pressed={view.watch.length === watchStates.length}
            onclick={() =>
              change({ watch: watchStates.map(([state]) => state) })}
            >All</button
          >
          {#each watchStates as [state, label] (state)}
            <button
              type="button"
              class={chipClass(narrowed(view.watch, watchStates, state))}
              aria-pressed={narrowed(view.watch, watchStates, state)}
              onclick={() =>
                change({ watch: choose(view.watch, watchStates, state) })}
              >{label}
              {#if counts}<span class="opacity-60">{counts.watch[state]}</span
                >{/if}</button
            >
          {/each}
        </div>
        <p class="mt-4 mb-1.5 text-xs text-muted">Availability</p>
        <div class="flex flex-wrap gap-1.5">
          <button
            type="button"
            class={chipClass(
              view.availability.length === availabilities.length,
            )}
            aria-pressed={view.availability.length === availabilities.length}
            onclick={() =>
              change({ availability: availabilities.map(([state]) => state) })}
            >All</button
          >
          {#each availabilities as [state, label] (state)}
            <button
              type="button"
              class={chipClass(
                narrowed(view.availability, availabilities, state),
              )}
              aria-pressed={narrowed(view.availability, availabilities, state)}
              onclick={() =>
                change({
                  availability: choose(
                    view.availability,
                    availabilities,
                    state,
                  ),
                })}
              >{label}
              {#if counts}<span class="opacity-60"
                  >{counts.availability[state]}</span
                >{/if}</button
            >
          {/each}
        </div>
        {#if shows}
          <p class="mt-4 mb-1.5 text-xs text-muted">Series</p>
          <div class="flex flex-wrap gap-1.5">
            {#each seriesStates as [state, label] (state)}
              <button
                type="button"
                class={chipClass(view.series === state)}
                aria-pressed={view.series === state}
                onclick={() => change({ series: state })}
                >{label}
                {#if counts?.series && state !== 'any'}<span class="opacity-60"
                    >{counts.series[state]}</span
                  >{/if}</button
              >
            {/each}
          </div>
        {/if}
        {#if genreOptions.length}
          <label
            class="mt-4 block text-xs text-muted"
            for={`library-genre-${domain}`}>Genre</label
          >
          <select
            id={`library-genre-${domain}`}
            value={view.genre}
            onchange={(event) => change({ genre: event.currentTarget.value })}
            class={selectClass}
          >
            <option value="">All genres</option>
            {#each genreOptions as genre (genre)}
              <option value={genre}>{genre}</option>
            {/each}
          </select>
        {/if}
        {#if collections.length}
          <label
            class="mt-4 block text-xs text-muted"
            for={`library-collection-${domain}`}>Collection</label
          >
          <select
            id={`library-collection-${domain}`}
            bind:value={collection}
            onchange={collectionChanged}
            class={selectClass}
          >
            <option value="">All movies</option>
            {#each collections as group (group.id)}
              <option value={String(group.id)}
                >{group.name} ({group.count})</option
              >
            {/each}
          </select>
        {/if}
      </Popover.ContentStatic>
    </Popover.Root>
  </div>
{/if}
