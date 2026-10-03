<script lang="ts">
  import { hasNativeAccess } from './lib/services/presentation';
  import Notice from './lib/ui/Notice.svelte';
  import ActivitySettings from './lib/ActivitySettings.svelte';
  import FormField from './lib/ui/FormField.svelte';
  import { formControlClass } from './lib/ui/styles';
  import AuthLayout from './lib/ui/AuthLayout.svelte';
  import Button from './lib/ui/Button.svelte';
  import Panel from './lib/ui/Panel.svelte';
  import { inlineFormClass, rowClass, statsClass } from './lib/ui/styles';
  import { onMount, tick } from 'svelte';
  import { Accordion } from 'bits-ui';
  import Sidebar from './lib/ui/Sidebar.svelte';
  import NavigationDrawer from './lib/ui/NavigationDrawer.svelte';
  import SettingsLayout from './lib/ui/SettingsLayout.svelte';
  import { connectAttention, refreshAttention } from './lib/attention';
  import WindowTitlebar from './lib/ui/WindowTitlebar.svelte';
  import {
    appearance,
    appearanceError,
    loadAppearance,
    refreshAppearance,
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
  const collapsed = $derived(compact ? false : $appearance.sidebar_collapsed);
  async function toggleSidebar() {
    const animate = sidebarMotion.capture(shell, main);
    settleLayoutMotions();
    updateAppearance({ sidebar_collapsed: !$appearance.sidebar_collapsed });
    await tick();
    syncMediaLayouts();
    animate();
  }
  import LibraryView from './lib/LibraryView.svelte';
  import PlaybackSettings from './lib/PlaybackSettings.svelte';
  import SegmentSettings from './lib/SegmentSettings.svelte';
  import TimezoneSelect from './lib/TimezoneSelect.svelte';
  import AdminOperations from './lib/AdminOperations.svelte';
  import BackupSettings from './lib/BackupSettings.svelte';
  import ProductVersion from './lib/ProductVersion.svelte';
  import { isServerUpdateInterruption } from './lib/server-updates';
  import ProductUpdatePreferences from './lib/ProductUpdatePreferences.svelte';
  import ServerTools from './lib/ServerTools.svelte';
  import DesktopVersion from './lib/DesktopVersion.svelte';
  import DesktopUpdatePreferences from './lib/DesktopUpdatePreferences.svelte';
  import WebUpdateReload from './lib/WebUpdateReload.svelte';
  import { connectDesktopUpdates } from './lib/desktop-updates';
  import UserAdministration from './lib/UserAdministration.svelte';
  import Player from './lib/Player.svelte';
  import MusicPlayer from './lib/MusicPlayer.svelte';
  import MpvSettings from './lib/MpvSettings.svelte';
  import { connectTools } from './lib/providers/tools-events';
  import NativePlayer from './lib/NativePlayer.svelte';
  import Discover from './lib/seerr/Discover.svelte';
  import Home from './lib/Home.svelte';
  import Playlists from './lib/Playlists.svelte';
  import History from './lib/History.svelte';
  import StatisticsView from './lib/statistics/StatisticsView.svelte';
  import UserPreferences from './lib/UserPreferences.svelte';
  import AutoSaveForm from './lib/ui/AutoSaveForm.svelte';
  import PasswordSettings from './lib/PasswordSettings.svelte';
  import QuickConnect from './lib/QuickConnect.svelte';
  import OnlineAccounts from './lib/OnlineAccounts.svelte';
  import OnlineSettings from './lib/OnlineSettings.svelte';
  import ProviderView from './lib/providers/ProviderView.svelte';
  import ServicesSettings from './lib/ServicesSettings.svelte';
  import ManagerOwnership from './lib/ManagerOwnership.svelte';
  import RetentionSettings from './lib/RetentionSettings.svelte';
  import { persistQueue } from './lib/media-state';
  import { invoke } from '@tauri-apps/api/core';
  import type { MediaChoice } from './lib/playback';
  import { youtubeVideoIdFromInput } from './lib/providers/youtube-video-input';
  import {
    providers,
    loadProviders,
    providerEnabled,
  } from './lib/providers/availability';
  let playing = $state<MediaChoice | null>(null);
  let playbackRequest = 0;
  let mediaRevision = $state(0),
    focusId = $state<string | undefined>(undefined);
  import {
    House,
    Compass,
    Film,
    Tv,
    Music,
    Play,
    Radio,
    Menu,
  } from '@lucide/svelte';
  import {
    api,
    ApiError,
    Events,
    type User,
    type Job,
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
  let timeFormat = $state<'12h' | '24h'>('24h'),
    serverTimeFormat = $state<'12h' | '24h'>('24h'),
    timezone = $state('UTC'),
    health = $state<{
      version: string;
      controller: boolean;
      cache_free_bytes: number;
    } | null>(null);
  const sections = [
    { name: 'Home', icon: House },
    { name: 'Discover', icon: Compass },
    { name: 'Movies', icon: Film },
    { name: 'Shows', icon: Tv },
    { name: 'Music', icon: Music },
    { name: 'Playlists', icon: Music },
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
      const requested = /^#discover\/(movie|tv)\/\d+$/.test(location.hash)
        ? 'Discover'
        : query.has('youtube_link')
          ? 'YouTube'
          : query.get('section');
      const restored = requested ?? saved?.section;
      const destination = restored === 'History' ? 'Statistics' : restored;
      if (
        [
          ...sections.map((item) => item.name),
          'Settings',
          'Statistics',
        ].includes(destination)
      )
        section = providerEnabled(destination) ? destination : 'Home';
      if (typeof saved?.settingsSection === 'string')
        settingsSection = saved.settingsSection;
      if (settingsSection === 'library') settingsSection = 'services';
      if (settingsSection === 'server-updates') settingsSection = 'server';
      if (desktop && settingsSection === 'updates')
        settingsSection = 'connection';
      if (!desktop && ['mpv', 'connection'].includes(settingsSection))
        settingsSection = 'account';
      if (!desktop && settingsSection === 'updates')
        settingsSection = user.role === 'admin' ? 'server' : 'account';
    } catch {
      /* Ignore an obsolete device preference. */
    }
    if (!providerEnabled(section)) section = 'Home';
    if (
      settingsSection === 'online' &&
      !Object.values($providers).some(Boolean)
    )
      settingsSection = 'account';
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
  const attentionUserId = $derived(user?.id);
  $effect(() => {
    const id = attentionUserId;
    if (id) return connectAttention(id);
  });
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
  async function checkServer() {
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
      return false;
    }
    setup = (await api<{ setup_required: boolean }>('/setup')).setup_required;
    return true;
  }
  async function boot() {
    loading = true;
    let reconnecting = false;
    for (;;) {
      try {
        await initializeTransport();
        serverAddress = serverUrl();
        if (!(await checkServer())) break;
        if (!setup) {
          try {
            user = (await api<{ user: User }>('/auth/me')).user;
          } catch (e) {
            if (!(e instanceof ApiError) || e.status !== 401) throw e;
            user = null;
          }
        }
        if (user) {
          if (returnToService()) return;
          await Promise.all([
            loadAppearance(user.id),
            loadDisplayPreferences(),
            loadProviders(),
          ]);
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
        if (
          (e instanceof ApiError && e.code === 'maintenance') ||
          (reconnecting && (!(e instanceof ApiError) || e.status >= 500))
        ) {
          reconnecting = true;
          await new Promise((resolve) => setTimeout(resolve, 1000));
          continue;
        }
        error = String(e);
      }
      break;
    }
    loading = false;
  }
  async function loadDisplayPreferences() {
    const value = await api<{
      timezone: string;
      time_format: '12h' | '24h';
    }>('/me/preferences');
    timeFormat = value.time_format;
    if (user) user = { ...user, timezone: value.timezone };
  }
  function startEvents() {
    events?.close();
    events = new Events(
      (event) => {
        if (
          ['online.configuration.changed', 'server.reconnected'].includes(
            event.kind,
          )
        ) {
          void loadProviders().catch((e) => (error = String(e)));
          accountRevision++;
          void refreshAttention();
        }
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
        if (
          [
            'attention.changed',
            'product.changed',
            'catalog.changed',
            'online.account.changed',
            'acquisition.changed',
            'jobs.changed',
            'managers.changed',
            'support.changed',
            'service-updates.changed',
            'tools.changed',
          ].includes(event.kind)
        )
          void refreshAttention();
        if (event.kind === 'server.reconnected') {
          void refreshAttention();
          mediaRevision++;
          catalogRevision++;
          accountRevision++;
          if (section === 'Settings') void loadSettings();
        }
        if (['tools.changed', 'server.reconnected'].includes(event.kind))
          window.dispatchEvent(new Event('thelxinoe-tools'));
        if (['product.changed', 'server.reconnected'].includes(event.kind))
          window.dispatchEvent(new Event('thelxinoe-product-update'));
        if (event.kind === 'online.account.changed') accountRevision++;
        if (
          event.kind === 'preferences.changed' ||
          event.kind === 'server.settings.changed' ||
          event.kind === 'account.profile.changed'
        ) {
          preferencesRevision++;
          if (event.kind === 'preferences.changed') {
            const preference = event.payload as {
              timezone: string;
              time_format: '12h' | '24h';
            };
            timeFormat = preference.time_format;
            if (user) user = { ...user, timezone: preference.timezone };
          }
          if (event.kind === 'server.settings.changed') {
            const settings = event.payload as {
              timezone: string;
              time_format: '12h' | '24h';
            };
            timezone = settings.timezone;
            serverTimeFormat = settings.time_format;
            void loadDisplayPreferences().catch((e) => (error = String(e)));
          }
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
      (value) => {
        connected = value;
        if (value && user) void refreshAppearance(user.id);
      },
    );
    events.connect();
  }
  async function authenticate() {
    await act(async () => {
      if (desktop) {
        await changeServer(serverAddress);
        serverAddress = serverUrl();
      }
      if (!(await checkServer())) return;
      if (setup && password !== passwordConfirmation) {
        error = passwordConfirmation
          ? 'Passwords do not match.'
          : 'Confirm your password to create the administrator account.';
        return;
      }
      user = (
        await api<{ user: User }>(setup ? '/setup' : '/auth/login', 'POST', {
          username,
          password,
        })
      ).user;
      password = '';
      passwordConfirmation = '';
      setup = false;
      if (returnToService()) return;
      await Promise.all([
        loadAppearance(user.id),
        loadDisplayPreferences(),
        loadProviders(),
      ]);
      restoreNavigation();
      startEvents();
    });
  }
  function returnToService() {
    const kind = new URLSearchParams(location.search).get('service');
    if (!desktop && user?.role === 'admin' && hasNativeAccess(kind)) {
      location.replace(`/services/${kind}`);
      return true;
    }
    return false;
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
      error = '';
      const result = await api<{ items: Session[]; current: string }>(
        '/auth/sessions',
      );
      sessions = result.items;
      currentSession = result.current;
      if (user?.role === 'admin') {
        users = (await api<{ items: User[] }>('/users')).items;
        jobs = (await api<{ items: Job[] }>('/admin/jobs')).items;
        health = await api('/admin/health');
        const settings = await api<{
          timezone: string;
          time_format: '12h' | '24h';
        }>('/admin/settings');
        timezone = settings.timezone;
        serverTimeFormat = settings.time_format;
      }
    } catch (e) {
      if (!isServerUpdateInterruption(e))
        error = e instanceof Error ? e.message : String(e);
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
    if (location.hash.startsWith('#discover/'))
      history.replaceState(
        history.state,
        '',
        `${location.pathname}${location.search}`,
      );
    section = name;
    error = '';
    if (name === 'Settings') await loadSettings();
  }
  function playYoutubeLink(event: MouseEvent) {
    if (
      !user ||
      !$providers.youtube ||
      event.defaultPrevented ||
      event.button !== 0 ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      event.altKey ||
      !(event.target instanceof Element)
    )
      return;
    const link = event.target.closest('a[href]');
    if (!(link instanceof HTMLAnchorElement) || link.hasAttribute('download'))
      return;
    const videoId = youtubeVideoIdFromInput(link.href);
    if (!videoId) return;
    event.preventDefault();
    void playMedia(
      {
        id: `youtube:${videoId}`,
        title:
          link.dataset.playbackTitle ||
          link.textContent?.trim() ||
          'YouTube video',
        position: 0,
      },
      true,
    );
  }
  async function playMedia(choice: MediaChoice, resolveYoutube = false) {
    if (!providerEnabled(choice.id)) return;
    const request = ++playbackRequest;
    const owner = user?.id;
    await act(async () => {
      if (resolveYoutube)
        await api('/online/youtube/resolve', 'POST', {
          video_id: choice.id.slice('youtube:'.length),
        });
      const next = await persistQueue(choice);
      if (request !== playbackRequest || user?.id !== owner) return;
      playing = next;
      if (!desktop) {
        await tick();
        workspace?.scrollTo({
          top: 0,
          behavior: prefersReducedMotion() ? 'instant' : 'smooth',
        });
      }
    });
  }
  $effect(() => {
    void $providers;
    if (!navigationReady || !user) return;
    if (!providerEnabled(section)) void navigate('Home');
    if (
      settingsSection === 'online' &&
      !Object.values($providers).some(Boolean)
    )
      settingsSection = 'account';
    if (playing && !providerEnabled(playing.id)) {
      playbackRequest++;
      if (desktop)
        void invoke('mpv_command', { command: 'stop', value: null }).catch(
          (e) => (error = String(e)),
        );
      playing = null;
    }
  });
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
    const updateConnection = connectDesktopUpdates();
    const toolsConnection = desktop
      ? connectTools()
      : Promise.resolve(() => {});
    const media = window.matchMedia('(max-width: 720px)');
    const resize = () => {
      compact = media.matches;
      mobileNavOpen = false;
    };
    resize();
    media.addEventListener('change', resize);
    const incompatible = (event: Event) => {
      updateRequired = (event as CustomEvent<string>).detail;
      events?.close();
    };
    const openSettings = (event: Event) => {
      if (
        (event as CustomEvent<string>).detail === 'server' &&
        user?.role === 'admin'
      ) {
        settingsSection = 'server';
        void navigate('Settings');
      }
    };
    window.addEventListener('thelxinoe-open-settings', openSettings);
    window.addEventListener('thelxinoe-update-required', incompatible);
    void boot().then(async () => {
      if (!desktop) return;
      await tick();
      await document.fonts.ready;
      await Promise.all(
        Array.from(
          document.querySelectorAll<HTMLImageElement>('img[src="/icon.svg"]'),
          (image) => image.decode().catch(() => {}),
        ),
      );
      try {
        await invoke('finish_startup');
      } catch (e) {
        error = String(e);
      }
    });
    return () => {
      void toolsConnection.then((disconnect) => disconnect());
      void updateConnection.then((disconnect) => disconnect());
      window.removeEventListener('thelxinoe-update-required', incompatible);
      window.removeEventListener('thelxinoe-open-settings', openSettings);
      events?.close();
      clearTimeout(settingsTimer);
      sidebarMotion.destroy();
      media.removeEventListener('change', resize);
    };
  });
</script>

<svelte:document onclick={playYoutubeLink} />

{#if desktop}<WindowTitlebar />{/if}
<WebUpdateReload playing={Boolean(playing)} admin={user?.role === 'admin'} />
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
    {#if desktop}<div class={statsClass}><DesktopVersion /></div>
      <FormField
        >Server address<input
          class={formControlClass}
          bind:value={serverAddress}
        /></FormField
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
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void authenticate();
      }}
    >
      {#if desktop}<FormField
          >Server address<input
            class={formControlClass}
            bind:value={serverAddress}
            placeholder="https://media.example.com"
            required
          /></FormField
        >{/if}
      <FormField
        >Username<input
          {@attach (element) => element.focus()}
          class={formControlClass}
          bind:value={username}
          required
          autocomplete="username"
        /></FormField
      >
      <FormField
        >Password<input
          class={formControlClass}
          bind:value={password}
          type="password"
          required
          minlength={setup ? 8 : 1}
          autocomplete={setup ? 'new-password' : 'current-password'}
        /></FormField
      >
      {#if setup}<FormField
          >Confirm password<input
            class={formControlClass}
            bind:value={passwordConfirmation}
            type="password"
            required
            minlength="8"
            autocomplete="new-password"
          /></FormField
        >
        <p class="text-[11px] text-muted">
          Use at least 8 characters. You can create other users after setup.
        </p>{/if}
      {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
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
    {#snippet navigation(currentUser: User)}
      <Sidebar
        {section}
        {collapsed}
        user={currentUser}
        navigate={(name) => void navigate(name)}
        toggle={() => void toggleSidebar()}
        logout={() => void logout()}
        onClose={compact ? () => (mobileNavOpen = false) : undefined}
      />
    {/snippet}
    {#if compact}
      {#if mobileNavOpen}
        <NavigationDrawer onClose={() => (mobileNavOpen = false)}>
          {@render navigation(user)}
        </NavigationDrawer>
      {/if}
    {:else}
      {@render navigation(user)}
    {/if}
    <main
      class="content flex min-w-0 flex-1 flex-col overflow-hidden"
      bind:this={main}
    >
      <header
        class="page-header flex shrink-0 items-center gap-5 border-b border-line bg-background/88 px-6 py-2 narrow:px-4 compact:gap-2.5 compact:px-3 compact:py-1.5"
      >
        {#if compact}
          <Button
            variant="ghost"
            size="icon"
            class="size-11 shrink-0"
            aria-label="Open navigation"
            aria-haspopup="dialog"
            aria-controls="mobile-navigation"
            aria-expanded={mobileNavOpen}
            onclick={() => (mobileNavOpen = true)}
            ><Menu class="size-5" /></Button
          >
        {/if}
        <div class="mr-auto min-w-0">
          <h1
            class="m-0 text-[18px] leading-6 compact:text-[17px]"
            data-sidebar-resize="x-pos"
          >
            {section}
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
        {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
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
              <UserPreferences
                {user}
                revision={preferencesRevision}
                avatarChanged={(id, avatar) => {
                  if (user?.id === id) user = { ...user, avatar };
                }}
                changed={(zone, format) => {
                  timeFormat = format;
                  if (user) user = { ...user, timezone: zone };
                }}
              />
              <PasswordSettings changed={() => void loadSettings()} />{/if}
            {#if settingsSection === 'online'}<OnlineAccounts
                revision={accountRevision}
                navigate={(name) => void navigate(name)}
                configureProviders={user.role === 'admin'
                  ? () => (settingsSection = 'providers')
                  : undefined}
              />{/if}
            {#if settingsSection === 'playback'}<PlaybackSettings />
              <SegmentSettings />
            {/if}
            {#if settingsSection === 'devices'}<QuickConnect
                username={user.username}
              />{/if}
            {#if desktop && settingsSection === 'mpv'}<MpvSettings />{/if}
            {#if desktop && settingsSection === 'connection'}<Panel>
                <h2>Desktop</h2>
                <div class={statsClass}><DesktopVersion /></div>
                <DesktopUpdatePreferences />
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
                  <FormField
                    >Server address<input
                      class={formControlClass}
                      bind:value={serverAddress}
                      required
                    /></FormField
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
                        ).toLocaleString(undefined, {
                          timeZone: user.timezone,
                          hour12: timeFormat === '12h',
                        })}{session.id === currentSession
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
              {#if settingsSection === 'analysis'}<SegmentSettings admin />{/if}
              {#if settingsSection === 'backups'}<BackupSettings
                  {timezone}
                  {timeFormat}
                />{/if}
              {#if settingsSection === 'providers'}<OnlineSettings />{/if}
              {#if settingsSection === 'services'}<ServicesSettings
                  {timeFormat}
                />{/if}
              {#if settingsSection === 'services'}<ManagerOwnership />{/if}
              {#if settingsSection === 'retention'}<RetentionSettings
                  {timezone}
                  {timeFormat}
                />{/if}
              {#if settingsSection === 'server'}<Panel>
                  <div class="flex items-center justify-between gap-3">
                    <h2>Server status</h2>
                  </div>
                  <div
                    class="grid divide-y divide-line [&>div]:py-3 [&_strong]:text-base [&_strong]:font-medium [&_small]:mt-1 [&_small]:block [&_small]:text-xs [&_small]:text-muted"
                  >
                    <ProductVersion installed={health?.version ?? '—'} />
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
                </Panel>
                <Panel
                  ><h2>Display defaults</h2>
                  <AutoSaveForm
                    label="Server display defaults"
                    class={inlineFormClass}
                    disabled={busy}
                    value={{ timezone, time_format: serverTimeFormat }}
                    onRevert={(previous) => {
                      timezone = previous.timezone;
                      serverTimeFormat = previous.time_format;
                    }}
                    onsave={(submitted) =>
                      api('/admin/settings', 'PUT', submitted)}
                  >
                    <TimezoneSelect
                      label="Server default timezone"
                      bind:value={timezone}
                      disabled={busy}
                    />
                    <FormField
                      >Server default time format<select
                        class={formControlClass}
                        bind:value={serverTimeFormat}
                        disabled={busy}
                        ><option value="24h">24-hour</option><option value="12h"
                          >12-hour</option
                        ></select
                      ></FormField
                    >
                  </AutoSaveForm>
                </Panel>
                <Panel
                  ><h2>Server updates</h2>
                  <ProductUpdatePreferences /></Panel
                >
                <Panel class="settings-wide"><ServerTools /></Panel>
                <AdminOperations {timezone} {timeFormat} />
              {/if}
              {#if settingsSection === 'people'}<Panel>
                  <h2>Add user</h2>
                  <form
                    class={inlineFormClass}
                    aria-label="Create user"
                    onsubmit={(e) => {
                      e.preventDefault();
                      void createUser();
                    }}
                  >
                    <FormField
                      >Username<input
                        class={formControlClass}
                        bind:value={newUsername}
                        required
                        autocomplete="off"
                      /></FormField
                    ><FormField
                      >Password<input
                        class={formControlClass}
                        bind:value={newPassword}
                        type="password"
                        required
                        minlength="8"
                        autocomplete="new-password"
                      /></FormField
                    ><FormField
                      >Role<select class={formControlClass} bind:value={newRole}
                        ><option value="user">User</option><option value="admin"
                          >Administrator</option
                        ></select
                      ></FormField
                    ><Button size="form" type="submit" disabled={busy}
                      >Add user</Button
                    >
                  </form>
                </Panel>
                <Panel class="settings-wide"
                  ><h2>User access</h2>
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
              {#if settingsSection === 'jobs'}<ActivitySettings
                  {jobs}
                  checkpoint={() =>
                    act(async () => {
                      await api('/admin/jobs', 'POST', {
                        key: crypto.randomUUID(),
                      });
                      await loadSettings();
                    })}
                />{/if}
            {/if}
          </SettingsLayout>
        {:else if section === 'Home'}
          <Home
            {user}
            revision={mediaRevision}
            {accountRevision}
            {playing}
            play={playMedia}
            navigate={(name) => void navigate(name)}
            details={(id, kind) => {
              void navigate(
                ['artist', 'album', 'track'].includes(kind)
                  ? 'Music'
                  : ['show', 'season', 'episode'].includes(kind)
                    ? 'Shows'
                    : 'Movies',
              ).then(() => (focusId = id));
            }}
          />
        {:else if section === 'Discover'}
          <Discover
            {user}
            navigate={(name) => void navigate(name)}
            settings={() => {
              settingsSection = 'services';
              void navigate('Settings');
            }}
          />
        {:else if ['Movies', 'Shows', 'Music'].includes(section)}
          <LibraryView
            domain={section}
            admin={user.role === 'admin'}
            revision={catalogRevision}
            {scans}
            userId={user.id}
            timezone={user.timezone}
            {timeFormat}
            {focusId}
            play={(choice) => void playMedia(choice)}
          />
        {:else if section === 'Playlists'}<Playlists
            userId={user.id}
            revision={mediaRevision}
            play={(choice) => void playMedia(choice)}
          />
        {:else if section === 'Statistics'}<StatisticsView {user} />
        {:else if providerPage}<ProviderView
            admin={user.role === 'admin'}
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
