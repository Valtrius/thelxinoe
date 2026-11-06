<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { DropdownMenu, Popover } from 'bits-ui';
  import {
    ArrowDown,
    ArrowUp,
    ArrowUpDown,
    Ellipsis,
    RefreshCw,
    Search,
  } from '@lucide/svelte';
  import { appearance, updateAppearance } from '../appearance';
  import { getAnimationDuration } from '../motion';
  import { showToast } from '../providers/toasts';
  import { loadRoots, scanRoot } from '../title/catalog';
  import Button from '../ui/Button.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import Skeleton from '../ui/Skeleton.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import { eyebrowTextClass, toolbarMenuClass } from '../ui/styles';
  import { plural } from './library';
  import {
    kindOf,
    savedSort,
    sorts,
    viewKey,
    views,
    type ListSort,
    type View,
  } from './list-view';

  let {
    view,
    sort,
    query,
    counts,
    busy,
    admin,
    viewChanged,
    filtersChanged,
  }: {
    view: View;
    sort: ListSort;
    query?: string;
    counts: Partial<Record<string, number>>;
    busy: boolean;
    admin: boolean;
    viewChanged: (view: View, query: string) => void;
    filtersChanged: (query: string) => void;
  } = $props();
  const kind = $derived(kindOf(view));
  function sortChanged(next: ListSort) {
    updateAppearance({
      provider_preferences: savedSort(
        view,
        next,
        $appearance.provider_preferences,
      ),
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
      filtersChanged(search);
    }, 250);
  }
  onDestroy(() => clearTimeout(searchTimer));

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

<div
  in:fade={{ duration: getAnimationDuration() }}
  class="flex min-w-0 flex-1 items-center gap-1.5 compact:flex-wrap"
>
  <ExclusiveChoiceGroup
    choices={viewChoices}
    value={selected}
    ariaLabel="Music lists"
    tall
    resizeWithSidebar
    onChange={(next) => {
      selected = next;
      clearTimeout(searchTimer);
      viewChanged(next, search);
      updateAppearance({ provider_preferences: { [viewKey]: next } });
    }}
  />
  <div
    data-sidebar-resize="xy-width"
    data-sidebar-group
    class="relative flex h-9 max-w-90 min-w-0 flex-1 items-center border border-line bg-surface focus-within:border-line-strong compact:min-w-45"
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
        class={`absolute top-11 right-0 z-50 w-64 border border-line-strong bg-surface-strong p-3 shadow-xl compact:top-full ${toolbarMenuClass}`}
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
  {#if counts[kind] !== undefined}<span
      class="ml-1.5 text-muted narrow:hidden compact:hidden"
      >{plural(counts[kind], kind)}</span
    >{:else if busy}<Skeleton
      class="ml-1.5 w-16 narrow:hidden compact:hidden"
    />{/if}
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
