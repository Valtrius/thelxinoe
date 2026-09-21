<script lang="ts">
  import AuthLayout from './lib/ui/AuthLayout.svelte';
  import Button from './lib/ui/Button.svelte';
  import Panel from './lib/ui/Panel.svelte';
  import {
    inlineFormClass,
    rowClass,
    sectionHeadingClass,
    badgeClass,
    errorClass,
    statsClass,
  } from './lib/ui/styles';
  import { onMount, tick } from 'svelte';
  import { Accordion } from 'bits-ui';
  import Sidebar from './lib/ui/Sidebar.svelte';
  import SettingsLayout from './lib/ui/SettingsLayout.svelte';
  import WindowTitlebar from './lib/ui/WindowTitlebar.svelte';
  import AppearanceSettings from './lib/AppearanceSettings.svelte';
  import {
    appearance,
    appearanceError,
    loadAppearance,
    updateAppearance,
    resetAppearance,
    acceptAppearance,
    type Appearance,
  } from './lib/appearance';
  import { createSidebarMotion } from './lib/sidebar-motion';
  import { syncMediaLayouts } from './lib/card-grid-zoom';
  import { createLayoutMotion, settleLayoutMotions } from './lib/layout-motion';
  import { createCardGridWheelHandler } from './lib/card-grid-wheel';
  import { prefersReducedMotion } from './lib/layout-animation';
  import { get } from 'svelte/store';
  const mediaMotion = createLayoutMotion();
  function zoomWheel(node: HTMLElement) {
    const wheel = createCardGridWheelHandler({
      anchorAttribute: 'data-layout-key',
      motion: mediaMotion,
      getColumns: () => get(appearance).card_columns,
      setColumns: (card_columns) => updateAppearance({ card_columns }),
    });
    const handle = (event: WheelEvent) => {
      if (!providerPage) wheel(event);
    };
    node.addEventListener('wheel', handle, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', handle) };
  }
  let shell = $state<HTMLDivElement | null>(null);
  let main = $state<HTMLElement | null>(null);
  let workspace = $state<HTMLElement | null>(null);
  let workspaceHeight = $state(0);
  let compact = $state(false),
    mobileNavOpen = $state(false);
  let settingsSection = $state('account');
  const sidebarMotion = createSidebarMotion();
  const collapsed = $derived(
    compact ? !mobileNavOpen : $appearance.sidebar_collapsed,
  );
  async function toggleSidebar() {
    const animate = sidebarMotion.capture(shell, main);
    settleLayoutMotions();
    if (compact) mobileNavOpen = !mobileNavOpen;
    else
      updateAppearance({ sidebar_collapsed: !$appearance.sidebar_collapsed });
    await tick();
    syncMediaLayouts();
    animate();
  }
  import LibraryView from './lib/LibraryView.svelte';
  import MetadataSettings from './lib/MetadataSettings.svelte';
  import PlaybackSettings from './lib/PlaybackSettings.svelte';
  import SegmentSettings from './lib/SegmentSettings.svelte';
  import TimezoneSelect from './lib/TimezoneSelect.svelte';
  import AdminOperations from './lib/AdminOperations.svelte';
  import BackupSettings from './lib/BackupSettings.svelte';
  import ProductUpdates from './lib/ProductUpdates.svelte';
  import DesktopUpdates from './lib/DesktopUpdates.svelte';
  import UserAdministration from './lib/UserAdministration.svelte';
  import Player from './lib/Player.svelte';
  import MusicPlayer from './lib/MusicPlayer.svelte';
  import MpvSettings from './lib/MpvSettings.svelte';
  import { connectTools } from './lib/providers/tools-events';
  import NativePlayer from './lib/NativePlayer.svelte';
  import PersonalHome from './lib/PersonalHome.svelte';
  import Playlists from './lib/Playlists.svelte';
  import History from './lib/History.svelte';
  import UserPreferences from './lib/UserPreferences.svelte';
  import PasswordSettings from './lib/PasswordSettings.svelte';
  import ProfilePicture from './lib/ProfilePicture.svelte';
  import QuickConnect from './lib/QuickConnect.svelte';
  import OnlineAccounts from './lib/OnlineAccounts.svelte';
  import OnlineSettings from './lib/OnlineSettings.svelte';
  import ProviderView from './lib/providers/ProviderView.svelte';
  import ManagerSettings from './lib/ManagerSettings.svelte';
  import ManagerOwnership from './lib/ManagerOwnership.svelte';
  import SupportServices from './lib/SupportServices.svelte';
  import ManagedStack from './lib/ManagedStack.svelte';
  import ServiceUpdates from './lib/ServiceUpdates.svelte';
  import RetentionSettings from './lib/RetentionSettings.svelte';
  import Requests from './lib/Requests.svelte';
  import { persistQueue, type Card } from './lib/media-state';
  import { invoke } from '@tauri-apps/api/core';
  import type { MediaChoice } from './lib/playback';
  let playing = $state<MediaChoice | null>(null);
  let mediaRevision = $state(0),
    focusId = $state<string | undefined>(undefined);
  import {
    House,
    Film,
    Tv,
    Music,
    Play,
    Radio,
    Library,
    ShieldCheck,
    RefreshCw,
  } from '@lucide/svelte';
  import {
    api,
    Events,
    type User,
    desktop,
    initializeTransport,
    changeServer,
    serverUrl,
  } from './lib/api';
  type Session = {
    id: string;
    name: string;
    transport: string;
    last_seen: number;
  };
  type Job = { id: string; kind: string; state: string; error: string | null };
  let loading = $state(true),
    setup = $state(false),
    user = $state<User | null>(null),
    error = $state(''),
    busy = $state(false),
    connected = $state(false);
  let username = $state(''),
    password = $state(''),
    passwordConfirmation = $state(''),
    section = $state(
      new URLSearchParams(location.search).has('youtube_link') ||
        new URLSearchParams(location.search).get('section') === 'YouTube'
        ? 'YouTube'
        : new URLSearchParams(location.search).get('section') === 'Twitch'
          ? 'Twitch'
          : 'Home',
    );
  const providerPage = $derived(
    ['YouTube', 'Twitch', 'Kick'].includes(section),
  );
  let sessions = $state<Session[]>([]),
    users = $state<User[]>([]),
    jobs = $state<Job[]>([]),
    currentSession = $state('');
  let newUsername = $state(''),
    newPassword = $state(''),
    newRole = $state<'admin' | 'user'>('user');
  let expandedUser = $state('');
  let preferencesRevision = $state(0);
  let timezone = $state('UTC'),
    health = $state<{
      version: string;
      controller: boolean;
      cache_free_bytes: number;
    } | null>(null);
  const sections = [
    { name: 'Home', icon: House },
    { name: 'Movies', icon: Film },
    { name: 'Shows', icon: Tv },
    { name: 'Music', icon: Music },
    { name: 'Playlists', icon: Music },
    { name: 'History', icon: Library },
    { name: 'Requests', icon: Library },
    { name: 'YouTube', icon: Play },
    { name: 'Twitch', icon: Radio },
    { name: 'Kick', icon: Radio },
  ];
  let events: Events | undefined;
  let navigationReady = $state(false);
  function restoreNavigation() {
    if (!user) return;
    const key = `thelxinoe:${serverUrl()}:${user.id}:navigation`;
    try {
      const saved = JSON.parse(localStorage.getItem(key) ?? 'null');
      const query = new URLSearchParams(location.search);
      const requested = query.has('youtube_link')
        ? 'YouTube'
        : query.get('section');
      const destination = requested ?? saved?.section;
      if (
        [
          ...sections.map((item) => item.name),
          'Settings',
          'Statistics',
        ].includes(destination)
      )
        section = destination;
      if (typeof saved?.settingsSection === 'string')
        settingsSection = saved.settingsSection;
      if (!desktop && ['mpv', 'connection'].includes(settingsSection))
        settingsSection = 'account';
      if (!desktop && settingsSection === 'updates')
        settingsSection = user.role === 'admin' ? 'server-updates' : 'account';
    } catch {
      /* Ignore an obsolete device preference. */
    }
    navigationReady = true;
    if (section === 'Settings') void loadSettings();
  }
  $effect(() => {
    if (navigationReady && user) {
      localStorage.setItem(
        `thelxinoe:${serverUrl()}:${user.id}:navigation`,
        JSON.stringify({ section, settingsSection }),
      );
      if (!desktop) {
        const url = new URL(location.href);
        if (
          url.searchParams.has('section') ||
          url.searchParams.has('youtube_link')
        ) {
          url.searchParams.delete('youtube_link');
          url.searchParams.set('section', section);
          window.history.replaceState(null, '', url);
        }
      }
    }
  });
  let settingsTimer: ReturnType<typeof setTimeout> | undefined,
    settingsLoading = false;
  let catalogRevision = $state(0);
  let notificationRevision = $state(0);
  let accountRevision = $state(0);
  let scans = $state<Record<string, { completed: number; total: number }>>({});
  let serverAddress = $state('');
  let updateRequired = $state('');
  async function act(fn: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await fn();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
  async function boot() {
    try {
      await initializeTransport();
      serverAddress = serverUrl();
      updateRequired = '';
      const contract = await api<{
        api_version: number;
        api_min?: number;
        api_max?: number;
      }>('/health');
      if (
        !Number.isInteger(contract.api_version) ||
        (contract.api_min ?? contract.api_version) > 1 ||
        (contract.api_max ?? contract.api_version) < 1
      ) {
        updateRequired =
          'This client cannot use the server’s API version. Update the client or connect to a compatible server.';
        return;
      }
      setup = (await api<{ setup_required: boolean }>('/setup')).setup_required;
      if (!setup) {
        try {
          user = (await api<{ user: User }>('/auth/me')).user;
        } catch {
          user = null;
        }
      }
      if (user) {
        await loadAppearance(user.id);
        restoreNavigation();
        startEvents();
        if (desktop) {
          const state = await invoke<{
            media_id: string;
            title: string;
            music: boolean;
            status: string;
          }>('mpv_state');
          if (state.media_id && !['stopped', 'failed'].includes(state.status))
            playing = {
              id: state.media_id,
              title: state.title,
              kind: state.music ? 'track' : 'movie',
              restore: true,
            };
        }
      }
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  function startEvents() {
    events?.close();
    events = new Events(
      (event) => {
        if (event.kind === 'online.download.progress')
          window.dispatchEvent(
            new CustomEvent('thelxinoe-download-progress', {
              detail: event.payload,
            }),
          );
        if (event.kind === 'appearance.changed')
          acceptAppearance(
            (event.payload as { appearance: Appearance }).appearance,
          );
        if (event.kind === 'notifications.changed') notificationRevision++;
        if (event.kind === 'online.account.changed') accountRevision++;
        if (
          event.kind === 'preferences.changed' ||
          event.kind === 'server.settings.changed' ||
          event.kind === 'account.profile.changed'
        ) {
          preferencesRevision++;
          if (event.kind === 'server.settings.changed')
            timezone = (event.payload as { timezone: string }).timezone;
          const userId = user?.id;
          void api<{ user: User }>('/auth/me')
            .then((result) => {
              if (user?.id === userId) user = result.user;
            })
            .catch((e) => (error = String(e)));
        }
        if (
          [
            'media-state.changed',
            'playback.changed',
            'playlists.changed',
            'catalog.changed',
            'youtube.changed',
            'online.account.changed',
            'online.configuration.changed',
            'online.download.changed',
          ].includes(event.kind)
        )
          mediaRevision++;
        if (event.kind === 'catalog.changed') {
          catalogRevision++;
          const root = (event.payload as { root_id?: string }).root_id;
          if (root) delete scans[root];
        }
        if (event.kind === 'catalog.scan.progress') {
          const scan = event.payload as {
            root_id: string;
            completed: number;
            total: number;
          };
          scans[scan.root_id] = scan;
        }
        if (section === 'Settings' && event.kind === 'jobs.changed') {
          clearTimeout(settingsTimer);
          settingsTimer = setTimeout(() => void loadSettings(), 250);
        }
      },
      (value) => (connected = value),
    );
    events.connect();
  }
  async function authenticate() {
    if (setup && password !== passwordConfirmation) {
      error = 'Passwords do not match.';
      return;
    }
    await act(async () => {
      user = (
        await api<{ user: User }>(setup ? '/setup' : '/auth/login', 'POST', {
          username,
          password,
        })
      ).user;
      password = '';
      passwordConfirmation = '';
      setup = false;
      await loadAppearance(user.id);
      restoreNavigation();
      startEvents();
    });
  }
  async function logout() {
    await act(async () => {
      await api('/auth/logout', 'POST');
      navigationReady = false;
      resetAppearance();
      playing = null;
      user = null;
      events?.close();
    });
  }
  async function loadSettings() {
    if (settingsLoading) return;
    settingsLoading = true;
    try {
      await act(async () => {
        const result = await api<{ items: Session[]; current: string }>(
          '/auth/sessions',
        );
        sessions = result.items;
        currentSession = result.current;
        if (user?.role === 'admin') {
          users = (await api<{ items: User[] }>('/users')).items;
          jobs = (await api<{ items: Job[] }>('/admin/jobs')).items;
          health = await api('/admin/health');
          timezone = (await api<{ timezone: string }>('/admin/settings'))
            .timezone;
        }
      });
    } finally {
      settingsLoading = false;
    }
  }
  $effect(() => {
    void section;
    if (workspace) workspace.scrollTop = 0;
  });
  async function navigate(name: string) {
    mobileNavOpen = false;
    focusId = undefined;
    section = name;
    error = '';
    if (name === 'Settings') await loadSettings();
  }
  function openMedia(item: Card) {
    section =
      item.kind === 'movie'
        ? 'Movies'
        : ['show', 'season', 'episode'].includes(item.kind)
          ? 'Shows'
          : 'Music';
    focusId = item.id;
  }
  async function playMedia(choice: MediaChoice) {
    await act(async () => {
      playing = await persistQueue(choice);
      if (!desktop) {
        await tick();
        workspace?.scrollTo({
          top: 0,
          behavior: prefersReducedMotion() ? 'instant' : 'smooth',
        });
      }
    });
  }
  async function createUser() {
    await act(async () => {
      await api('/users', 'POST', {
        username: newUsername,
        password: newPassword,
        role: newRole,
      });
      newUsername = '';
      newPassword = '';
      users = (await api<{ items: User[] }>('/users')).items;
    });
  }
  onMount(() => {
    document.body.classList.toggle('desktop-app', desktop);
    const toolsConnection = desktop
      ? connectTools()
      : Promise.resolve(() => {});
    const media = window.matchMedia('(max-width: 720px)');
    const resize = () => {
      compact = media.matches;
    };
    resize();
    media.addEventListener('change', resize);
    const incompatible = (event: Event) => {
      updateRequired = (event as CustomEvent<string>).detail;
      events?.close();
    };
    window.addEventListener('thelxinoe-update-required', incompatible);
    void boot();
    return () => {
      void toolsConnection.then((disconnect) => disconnect());
      window.removeEventListener('thelxinoe-update-required', incompatible);
      events?.close();
      clearTimeout(settingsTimer);
      sidebarMotion.destroy();
      media.removeEventListener('change', resize);
    };
  });
</script>

{#if desktop}<WindowTitlebar />{/if}
{#if loading}
  <AuthLayout card={false}>
    <img
      class="block size-10.5 object-contain"
      src="/icon.svg"
      alt="Thelxinoe"
      width="42"
      height="42"
    />
    <p>Connecting to your library…</p>
  </AuthLayout>
{:else if updateRequired}
  <AuthLayout>
    <h1>Update required</h1>
    <p>{updateRequired}</p>
    {#if desktop}<DesktopUpdates /><label
        >Server address<input bind:value={serverAddress} /></label
      ><Button
        size="form"
        variant="secondary"
        type="submit"
        onclick={() =>
          act(async () => {
            await changeServer(serverAddress);
            await boot();
          })}>Connect to server</Button
      >{:else}<Button
        size="form"
        variant="secondary"
        type="submit"
        onclick={() => location.reload()}>Reload current web app</Button
      >{/if}
  </AuthLayout>
{:else if !user}
  <AuthLayout>
    <img
      class="block size-10.5 object-contain"
      src="/icon.svg"
      alt="Thelxinoe"
      width="42"
      height="42"
    />
    <h1>{setup ? 'Welcome to Thelxinoe' : 'Welcome back'}</h1>
    <p class="text-muted">
      {setup
        ? 'Create the administrator account for your media server.'
        : 'Sign in to pick up where you left off.'}
    </p>
    {#if desktop}<label
        >Server address<input
          bind:value={serverAddress}
          placeholder="https://media.example.com"
        /></label
      ><Button
        size="form"
        variant="secondary"
        type="button"
        onclick={() =>
          act(async () => {
            await changeServer(serverAddress);
            await boot();
          })}>Connect to server</Button
      >{/if}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void authenticate();
      }}
    >
      <label
        >Username<input
          bind:value={username}
          required
          autocomplete="username"
        /></label
      >
      <label
        >Password<input
          bind:value={password}
          type="password"
          required
          minlength={setup ? 12 : 1}
          autocomplete={setup ? 'new-password' : 'current-password'}
        /></label
      >
      {#if setup}<label
          >Confirm password<input
            bind:value={passwordConfirmation}
            type="password"
            required
            minlength="12"
            autocomplete="new-password"
          /></label
        >
        <p class="text-[11px] text-muted">
          Use at least 12 characters. You can create other users after setup.
        </p>{/if}
      {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
      <Button size="form" type="submit" class="w-full" disabled={busy}
        >{busy
          ? 'Connecting…'
          : setup
            ? 'Create your server'
            : 'Sign in'}</Button
      >
    </form>
  </AuthLayout>
{:else}
  <div
    class="app-shell flex h-dvh overflow-hidden desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-32px)]"
    bind:this={shell}
  >
    <Sidebar
      {section}
      {collapsed}
      {user}
      {notificationRevision}
      navigate={(name) => void navigate(name)}
      toggle={() => void toggleSidebar()}
      logout={() => void logout()}
    />
    {#if compact && mobileNavOpen}<button
        class="sidebar-scrim fixed inset-0 z-60 bg-black/53"
        aria-label="Close navigation"
        onclick={() => (mobileNavOpen = false)}
      ></button>{/if}
    <main
      class={[
        'content flex min-w-0 flex-1 flex-col overflow-hidden',
        compact && mobileNavOpen && 'ml-18',
      ]}
      bind:this={main}
    >
      <header
        class="page-header flex shrink-0 items-center gap-5 border-b border-line bg-background/88 px-6 py-2 narrow:px-4 compact:gap-2.5 compact:px-3 compact:py-4"
      >
        <div class="mr-auto min-w-0">
          <h1
            class="m-0 text-[18px] leading-6 compact:text-[17px]"
            data-sidebar-resize="x-pos"
          >
            {section === 'Home' ? `Good to see you, ${user.username}` : section}
          </h1>
        </div>
        <span
          class="connection inline-flex items-center gap-1.75 text-[10px] whitespace-nowrap text-muted compact:text-[0px]"
          ><i
            class={[
              'size-1.25 rounded-full',
              connected ? 'bg-success' : 'bg-warning',
            ]}
            class:online={connected}
          ></i>{connected ? 'Connected' : 'Reconnecting'}</span
        >
      </header>
      <div
        class={[
          'workspace-scroll relative min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto [overflow-anchor:none]',
          providerPage
            ? 'pt-3 pr-0 pb-0 pl-3 scrollbar-stable'
            : section === 'Settings'
              ? 'p-0'
              : 'p-6 narrow:p-4 compact:p-3',
        ]}
        bind:this={workspace}
        bind:clientHeight={workspaceHeight}
        style:--workspace-height={`${workspaceHeight}px`}
        class:provider-workspace={providerPage}
        class:settings-workspace={section === 'Settings'}
        data-feed-scroll
        data-sidebar-resize="xy"
        data-sidebar-resize-origin
        use:mediaMotion.connect
        use:zoomWheel
      >
        {#if $appearanceError}<p class="text-muted" role="status">
            {$appearanceError}
          </p>{/if}
        {#if error}<p class={errorClass} role="alert">{error}</p>{/if}
        {#if playing && desktop}<NativePlayer
            choice={playing}
            closed={(closedChoice) => {
              if (playing === closedChoice) playing = null;
            }}
          />{:else if playing && playing.kind === 'track'}<MusicPlayer
            choice={playing}
            closed={() => (playing = null)}
          />{:else if playing}<div
            class={[
              'player-frame',
              providerPage
                ? '-mt-3 -ml-3'
                : section === 'Settings'
                  ? ''
                  : '-mx-6 -mt-6 narrow:-mx-4 narrow:-mt-4 compact:-mx-3 compact:-mt-3',
            ]}
          >
            <Player
              choice={playing}
              closed={() => (playing = null)}
              resizable
            />
          </div>{/if}
        {#if section === 'Settings'}
          <SettingsLayout {user} bind:active={settingsSection}>
            {#if settingsSection === 'account'}
              <ProfilePicture
                {user}
                changed={(id, avatar) => {
                  if (user?.id === id) user = { ...user, avatar };
                }}
              />
              <UserPreferences
                {user}
                revision={preferencesRevision}
                changed={(zone) => {
                  if (user) user = { ...user, timezone: zone };
                }}
              />
              <AppearanceSettings />
              <PasswordSettings changed={() => void loadSettings()} />{/if}
            {#if settingsSection === 'online'}<OnlineAccounts
                revision={accountRevision}
                navigate={(name) => void navigate(name)}
              />{/if}
            {#if settingsSection === 'playback'}<PlaybackSettings />
              <SegmentSettings />
            {/if}
            {#if settingsSection === 'devices'}<QuickConnect
                username={user.username}
              />{/if}
            {#if desktop && settingsSection === 'mpv'}<MpvSettings />{/if}
            {#if desktop && settingsSection === 'updates'}<DesktopUpdates
              />{/if}
            {#if desktop && settingsSection === 'connection'}<Panel>
                <h2>Server connection</h2>
                <form
                  class={inlineFormClass}
                  onsubmit={(e) => {
                    e.preventDefault();
                    void act(async () => {
                      await changeServer(serverAddress);
                      playing = null;
                      events?.close();
                      user = null;
                      await boot();
                    });
                  }}
                >
                  <label
                    >Server address<input
                      bind:value={serverAddress}
                      required
                    /></label
                  ><Button
                    size="form"
                    variant="secondary"
                    type="submit"
                    disabled={busy}>Change server</Button
                  >
                </form>
              </Panel>{/if}
            {#if settingsSection === 'devices'}<Panel>
                <h2>Your devices</h2>
                <p class="text-muted">
                  Revoke access to a browser or desktop at any time.
                </p>
                {#each sessions as session (session.id)}<div class={rowClass}>
                    <div>
                      <strong>{session.name}</strong><small
                        >{session.transport} · {new Date(
                          session.last_seen * 1000,
                        ).toLocaleString()}{session.id === currentSession
                          ? ' · This device'
                          : ''}</small
                      >
                    </div>
                    <Button
                      size="form"
                      variant="secondary"
                      type="submit"
                      disabled={busy}
                      onclick={() =>
                        act(async () => {
                          await api(`/auth/sessions/${session.id}`, 'DELETE');
                          if (session.id === currentSession) {
                            user = null;
                            events?.close();
                          } else await loadSettings();
                        })}>Revoke</Button
                    >
                  </div>{/each}
              </Panel>
            {/if}
            {#if user.role === 'admin'}
              {#if settingsSection === 'server-updates'}<ProductUpdates />{/if}
              {#if settingsSection === 'analysis'}<SegmentSettings admin />{/if}
              {#if settingsSection === 'backups'}<BackupSettings />{/if}
              {#if settingsSection === 'library'}<MetadataSettings />{/if}
              {#if settingsSection === 'providers'}<OnlineSettings />{/if}
              {#if settingsSection === 'services'}<ManagerSettings />{/if}
              {#if settingsSection === 'services'}<ManagerOwnership />{/if}
              {#if settingsSection === 'services'}<ManagedStack />{/if}
              {#if settingsSection === 'services'}<ServiceUpdates />{/if}
              {#if settingsSection === 'retention'}<RetentionSettings />{/if}
              {#if settingsSection === 'services'}<SupportServices />{/if}
              {#if settingsSection === 'server'}<Panel>
                  <h2><ShieldCheck size={20} /> Server</h2>
                  <div class={statsClass}>
                    <div>
                      <strong>{health?.version ?? '—'}</strong><small
                        >Product version</small
                      >
                    </div>
                    <div>
                      <strong
                        >{health?.controller
                          ? 'Connected'
                          : 'Unavailable'}</strong
                      ><small>Docker controller</small>
                    </div>
                    <div>
                      <strong
                        >{health
                          ? `${(health.cache_free_bytes / 1024 ** 3).toFixed(1)} GB`
                          : '—'}</strong
                      ><small>Cache space available</small>
                    </div>
                  </div>
                  <form
                    class={inlineFormClass}
                    onsubmit={(e) => {
                      e.preventDefault();
                      void act(async () => {
                        await api('/admin/settings', 'PUT', { timezone });
                      });
                    }}
                  >
                    <TimezoneSelect
                      label="Server default timezone"
                      bind:value={timezone}
                      disabled={busy}
                    />
                    <Button
                      size="form"
                      variant="secondary"
                      type="submit"
                      disabled={busy}>Save</Button
                    >
                  </form>
                  <p class="text-muted">
                    Used by everyone who has not chosen a personal display
                    timezone. Stored timestamps remain in UTC.
                  </p>
                </Panel>
                <AdminOperations />
              {/if}
              {#if settingsSection === 'people'}<Panel>
                  <h2>People</h2>
                  <form
                    class={inlineFormClass}
                    aria-label="Create user"
                    onsubmit={(e) => {
                      e.preventDefault();
                      void createUser();
                    }}
                  >
                    <label
                      >Username<input
                        bind:value={newUsername}
                        required
                        autocomplete="off"
                      /></label
                    ><label
                      >Password<input
                        bind:value={newPassword}
                        type="password"
                        required
                        minlength="12"
                        autocomplete="new-password"
                      /></label
                    ><label
                      >Role<select bind:value={newRole}
                        ><option value="user">User</option><option value="admin"
                          >Administrator</option
                        ></select
                      ></label
                    ><Button size="form" type="submit" disabled={busy}
                      >Add user</Button
                    >
                  </form>
                  <Accordion.Root type="single" bind:value={expandedUser}>
                    {#each users as person (person.id)}<UserAdministration
                        {person}
                        currentId={user.id}
                        changed={loadSettings}
                        close={() => (expandedUser = '')}
                      />{/each}
                  </Accordion.Root>
                </Panel>
              {/if}
              {#if settingsSection === 'audit'}<History {user} audit />{/if}
              {#if settingsSection === 'jobs'}<Panel>
                  <div class={sectionHeadingClass}>
                    <h2>Background jobs</h2>
                    <Button
                      size="form"
                      variant="secondary"
                      type="submit"
                      onclick={() =>
                        act(async () => {
                          await api('/admin/jobs', 'POST', {
                            key: crypto.randomUUID(),
                          });
                          await loadSettings();
                        })}><RefreshCw size={15} /> Run checkpoint</Button
                    >
                  </div>
                  {#each jobs as job (job.id)}<div class={rowClass}>
                      <span>{job.kind}</span><span class={badgeClass}
                        >{job.state}</span
                      >
                    </div>{:else}<p class="text-muted">
                      No background jobs yet.
                    </p>{/each}
                </Panel>
              {/if}
            {/if}
          </SettingsLayout>
        {:else if section === 'Home'}
          <PersonalHome
            revision={mediaRevision}
            open={openMedia}
            play={(choice) => void playMedia(choice)}
          />
          <div class={sectionHeadingClass}>
            <h2>Your collections</h2>
            <span class="text-muted">Built around you</span>
          </div>
          <div class="grid grid-cols-3 gap-3 compact:grid-cols-1">
            {#each sections.slice(1, 4) as item (item.name)}<button
                class="flex flex-col items-start gap-4 border border-line bg-surface p-6 text-left text-foreground transition-[transform,border-color] duration-200 hover:border-line-strong hover:[transform:translateY(-2px)] [&_svg]:text-accent [&_strong]:text-sm [&_strong]:leading-normal [&_strong]:font-[550] [&_span]:text-[11px] [&_span]:text-muted compact:flex-row compact:items-center compact:p-4 compact:[&_span]:ml-auto"
                onclick={() => navigate(item.name)}
                ><item.icon size={30} /><strong>{item.name}</strong><span
                  >Browse your collection →</span
                ></button
              >{/each}
          </div>
        {:else if ['Movies', 'Shows', 'Music'].includes(section)}
          <LibraryView
            domain={section}
            admin={user.role === 'admin'}
            revision={catalogRevision}
            {scans}
            userId={user.id}
            {focusId}
            play={(choice) => void playMedia(choice)}
          />
        {:else if section === 'Playlists'}<Playlists
            userId={user.id}
            revision={mediaRevision}
            play={(choice) => void playMedia(choice)}
          />
        {:else if section === 'Requests'}<Requests {user} />
        {:else if section === 'History' || section === 'Statistics'}<History
            {user}
            statistics={section === 'Statistics'}
          />
        {:else if providerPage}<ProviderView
            platform={section.toLowerCase() as 'youtube' | 'twitch' | 'kick'}
            userId={user.id}
            revision={mediaRevision}
            {playing}
            play={playMedia}
            settings={(tab) => {
              settingsSection =
                tab === 'providers' && user?.role !== 'admin' ? 'online' : tab;
              void navigate('Settings');
            }}
          />
        {/if}
      </div>
    </main>
  </div>
{/if}
