<script lang="ts">
  import type { Snippet } from 'svelte';
  import { Search } from '@lucide/svelte';
  import NavigationItem from './NavigationItem.svelte';
  import { followLink, routeHref, sectionRoute } from '../navigation';
  import AttentionDot from './AttentionDot.svelte';
  import { attention, attentionDescription } from '../attention';
  import { activity } from '../activity';
  import { formControlClass } from './styles';
  import type { SettingsGroup } from '../settings/sections';
  import type { SettingsSearchResult } from '../settings/search';
  let {
    groups,
    active = 'account',
    query = $bindable(''),
    results,
    children,
    subheader,
    openSettings,
  }: {
    groups: SettingsGroup[];
    active?: string;
    query?: string;
    // Set while a search is showing results instead of the active page.
    results?: SettingsSearchResult[];
    children: Snippet;
    subheader?: Snippet;
    openSettings: (name: string) => void;
  } = $props();
  let scroller = $state<HTMLElement>(),
    searchInput = $state<HTMLInputElement>();
  // Ctrl+K jumps to the search from anywhere on the page, unless a dialog or
  // fullscreen player is in front of it or the focused editor used the key.
  function focusSearch(event: KeyboardEvent) {
    if (
      event.key.toLowerCase() !== 'k' ||
      !event.ctrlKey ||
      event.altKey ||
      event.shiftKey ||
      event.metaKey ||
      event.isComposing ||
      event.defaultPrevented ||
      document.fullscreenElement ||
      document.querySelector('dialog[open]')
    )
      return;
    event.preventDefault();
    searchInput?.focus();
    searchInput?.select();
  }
  // The navigation and page header stay put; only the cards scroll. A new
  // section or search starts at the top. The scroller is positioned so
  // absolutely placed descendants (screen-reader text, badges) scroll with it
  // instead of stretching the workspace behind it.
  $effect(() => {
    void active;
    void results;
    scroller?.scrollTo({ top: 0 });
  });
  const page = $derived(
    groups.flatMap((group) => group.items).find((item) => item.id === active),
  );
  const title = $derived(
    results ? 'Search results' : (page?.label ?? 'Settings'),
  );
  const description = $derived(
    results
      ? results.length
        ? `Settings matching “${query.trim()}”, best match first.`
        : ''
      : (page?.description ?? ''),
  );
  // While searching, the navigation lists the pages with results.
  const filtered = $derived(
    results
      ? groups.map((group) => ({
          ...group,
          items: group.items.filter((item) =>
            results.some((result) => result.page.id === item.id),
          ),
        }))
      : groups,
  );
</script>

<svelte:window onkeydown={focusSearch} />

<div
  class="settings-layout mx-auto flex h-(--workspace-height) min-h-0 w-full max-w-520 items-start gap-8 px-8 pt-7 narrow:gap-5 narrow:px-5 compact:h-auto compact:min-h-full compact:flex-col compact:gap-6 compact:px-4 compact:py-4"
>
  <nav
    aria-label="Settings navigation"
    data-sidebar-resize="xy-pos"
    class="settings-navigation @container/settings-nav max-h-full w-49 shrink-0 space-y-6 overflow-y-auto py-1 pb-7 compact:max-h-none compact:w-full compact:space-y-4 compact:overflow-visible compact:pb-0"
  >
    <label class="relative block">
      <span class="sr-only">Find a setting</span>
      <Search
        size={14}
        class="pointer-events-none absolute top-3 left-2.5 text-muted"
        aria-hidden="true"
      />
      <input
        bind:this={searchInput}
        type="search"
        class={[formControlClass, 'pl-8 text-xs']}
        placeholder="Find a setting (Ctrl+K)"
        aria-keyshortcuts="Control+K"
        bind:value={query}
        onkeydown={(event) => {
          if (event.key === 'Escape' && query) {
            event.preventDefault();
            query = '';
          }
        }}
      />
    </label>
    {#each filtered as group (group.label)}
      {#if group.items.length}
        <div>
          <p
            class="mb-2 px-2 text-[10px] font-semibold tracking-widest text-muted uppercase"
          >
            {group.label}
          </p>
          <div
            class="grid gap-1 compact:grid-cols-3 compact:gap-1.5 compact:@max-[270px]/settings-nav:grid-cols-2"
          >
            {#each group.items as item (item.id)}
              <NavigationItem
                aria-label={item.label}
                description={attentionDescription(
                  $attention.filter((entry) => entry.target === item.id),
                  $activity.filter((entry) => entry.target === item.id),
                )}
                active={!results && active === item.id}
                href={routeHref(sectionRoute('Settings', item.id))}
                onclick={(event) => {
                  if (!followLink(event)) return;
                  event.preventDefault();
                  openSettings(item.id);
                  query = '';
                }}
                class="settings-nav-item flex min-h-10 items-center gap-2.5 border border-transparent px-2.5 py-2 text-xs aria-[current=page]:border-line-strong compact:min-h-11 compact:gap-1 compact:px-1 compact:text-[11px] [&_[data-nav-accent]]:hidden"
              >
                <item.icon
                  size={16}
                  strokeWidth={1.6}
                  class="shrink-0 compact:size-3.5"
                  aria-hidden="true"
                />
                <span class="min-w-0 leading-snug wrap-anywhere"
                  >{item.label}</span
                >
                <AttentionDot
                  items={$attention.filter((entry) => entry.target === item.id)}
                  activity={$activity.filter(
                    (entry) => entry.target === item.id,
                  )}
                />
              </NavigationItem>
            {/each}
          </div>
        </div>
      {/if}
    {/each}
  </nav>
  <div
    class="settings-content @container/settings flex min-h-0 min-w-0 flex-1 flex-col self-stretch compact:w-full compact:self-auto"
    data-page-region="settings"
    data-sidebar-resize="xy-pos"
    data-sidebar-resize-origin
  >
    <header
      class="settings-header mb-6 min-w-0 shrink-0 compact:mb-5"
      data-sidebar-resize="xy-pos"
    >
      <h1 class="text-[25px] font-semibold tracking-tight">{title}</h1>
      {#if description}
        <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted">
          {description}
        </p>
      {/if}
    </header>
    {#if subheader}<div class="shrink-0">{@render subheader()}</div>{/if}
    <div
      bind:this={scroller}
      class="settings-scroll relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-7 [scrollbar-gutter:stable] compact:overflow-visible compact:pb-0"
    >
      <div class="settings-panels settings-grid">
        {@render children()}
      </div>
    </div>
  </div>
</div>
