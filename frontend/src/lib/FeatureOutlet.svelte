<script lang="ts">
  import type { User } from './api';
  import type { AppRoute } from './navigation';
  import type { MediaChoice } from './playback';
  import { pages } from './pages';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import MediaSkeleton from './ui/MediaSkeleton.svelte';
  let {
    route,
    user,
    timeFormat,
    mediaRevision,
    catalogRevision,
    accountRevision,
    preferencesRevision,
    settingsRevision,
    scans,
    playing,
    play,
    navigate,
    openSettings,
    go,
    updateFilters,
    back,
    sessionEnded,
    switchServer,
    displayChanged,
    profileChanged,
  } = $props<{
    route: AppRoute;
    user: User;
    timeFormat: '12h' | '24h';
    mediaRevision: number;
    catalogRevision: number;
    accountRevision: number;
    preferencesRevision: number;
    settingsRevision: number;
    scans: Record<string, { completed: number; total: number }>;
    playing: MediaChoice | null;
    play: (choice: MediaChoice, resolveYoutube?: boolean) => Promise<void>;
    navigate: (name: string) => void;
    openSettings: (name: string, service?: string) => void;
    go: (route: AppRoute, replace?: boolean) => void;
    updateFilters: (route: AppRoute, replace?: boolean) => void;
    back: () => void;
    sessionEnded: () => void;
    switchServer: (address: string) => Promise<void>;
    displayChanged: (timezone: string, format: '12h' | '24h') => void;
    profileChanged: (id: string, avatar: string | null) => void;
  }>();
  let retry = $state(0);
</script>

{#snippet loading()}
  {#if route.section === 'Settings' || route.section === 'Playlists'}
    <ContentSkeleton
      label={`Loading ${route.section.toLowerCase()}`}
      variant={route.section === 'Settings' ? 'settings' : 'rows'}
    />
  {:else if route.section === 'Statistics'}
    <ContentSkeleton label="Loading statistics" variant="statistics" />
  {:else if route.section === 'Discover' && route.detail}
    <ContentSkeleton label="Loading details" variant="detail" />
  {:else}
    <MediaSkeleton
      label={`Loading ${route.section.toLowerCase()}`}
      heading={route.section === 'Home' ? 'Continue watching' : undefined}
      layout={route.section === 'Home'
        ? 'row'
        : route.section === 'Discover'
          ? 'posters'
          : 'grid'}
      shape={route.section === 'Music'
        ? 'square'
        : ['Home', 'YouTube', 'Twitch', 'Kick'].includes(route.section)
          ? 'landscape'
          : 'poster'}
    />
  {/if}
{/snippet}
{#snippet failed(error: unknown)}
  <Notice variant="error" role="alert"
    >{String(error)}
    <Button variant="secondary" size="form" onclick={() => retry++}
      >Retry page</Button
    >
  </Notice>
{/snippet}
{#key retry}
  {#if route.section === 'Home'}
    {#await pages.home()}{@render loading()}{:then { default: Home }}
      <Home
        {user}
        revision={mediaRevision}
        {accountRevision}
        {playing}
        {play}
        {navigate}
        details={(item, kind) =>
          go({
            section: ['artist', 'album', 'track'].includes(kind)
              ? 'Music'
              : ['show', 'season', 'episode'].includes(kind)
                ? 'Shows'
                : 'Movies',
            settings: 'account',
            item,
          })}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else if route.section === 'Settings'}
    {#await pages.settings()}{@render loading()}{:then { default: SettingsPage }}
      <SettingsPage
        {route}
        {user}
        {timeFormat}
        {preferencesRevision}
        {accountRevision}
        revision={settingsRevision}
        {navigate}
        {openSettings}
        {sessionEnded}
        {switchServer}
        {displayChanged}
        {profileChanged}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else if route.section === 'Discover'}
    {#await pages.discover()}{@render loading()}{:then { default: Discover }}
      <Discover
        {user}
        {navigate}
        settings={() => openSettings('services')}
        detail={route.detail}
        showRequests={route.requests}
        searchQuery={route.query}
        searchChanged={(query) =>
          updateFilters({ ...route, query, requests: false })}
        requestsChanged={(requests) =>
          go({ ...route, detail: undefined, requests, query: undefined })}
        openMedia={(detail) => go({ ...route, detail, requests: false })}
        {back}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else if ['Movies', 'Shows', 'Music'].includes(route.section)}
    {#await pages.library()}{@render loading()}{:then { default: LibraryView }}
      <LibraryView
        domain={route.section}
        admin={user.role === 'admin'}
        revision={catalogRevision}
        {scans}
        userId={user.id}
        timezone={user.timezone}
        {timeFormat}
        focusId={route.item}
        ancestors={route.ancestors}
        searchQuery={route.query}
        collectionId={route.collection}
        filtersChanged={(query, collection) =>
          updateFilters({ ...route, query, collection })}
        selectedChanged={(item, ancestors) => go({ ...route, item, ancestors })}
        {play}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else if route.section === 'Playlists'}
    {#await pages.playlists()}{@render loading()}{:then { default: Playlists }}
      <Playlists
        userId={user.id}
        revision={mediaRevision}
        {play}
        selectedId={route.playlist}
        selectedChanged={(playlist, replace) =>
          go({ ...route, playlist }, replace)}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else if route.section === 'Statistics'}
    {#await pages.statistics()}{@render loading()}{:then { default: StatisticsView }}
      <StatisticsView
        {user}
        {timeFormat}
        filters={route}
        filtersChanged={(filters) =>
          updateFilters({ ...route, ...filters }, false)}
      />
    {:catch error}{@render failed(error)}{/await}
  {:else}
    {#await pages.providers()}{@render loading()}{:then { default: ProviderView }}
      <ProviderView
        admin={user.role === 'admin'}
        platform={route.section.toLowerCase() as 'youtube' | 'twitch' | 'kick'}
        userId={user.id}
        revision={mediaRevision}
        watchlistId={route.watchlist}
        watchlistChanged={(watchlist) => go({ ...route, watchlist })}
        {playing}
        {play}
        settings={(name) =>
          openSettings(
            name === 'providers' && user.role !== 'admin' ? 'online' : name,
          )}
      />
    {:catch error}{@render failed(error)}{/await}
  {/if}
{/key}
