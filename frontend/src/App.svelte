<script lang="ts">
  import { hasNativeAccess } from './lib/services/presentation';
  import Notice from './lib/ui/Notice.svelte';
  import FormField from './lib/ui/FormField.svelte';
  import { formControlClass } from './lib/ui/styles';
  import AuthLayout from './lib/ui/AuthLayout.svelte';
  import Button from './lib/ui/Button.svelte';
  import SignIn from './lib/SignIn.svelte';
  import Recovery from './lib/Recovery.svelte';
  import DesktopApproval from './lib/DesktopApproval.svelte';
  import AuthVerification from './lib/AuthVerification.svelte';
  import { cancelVerification, type AuthOptions } from './lib/authentication';
  import { statsClass } from './lib/ui/styles';
  import { onMount, tick } from 'svelte';
  import { SvelteURL } from 'svelte/reactivity';
  import { players, preloadPage } from './lib/pages';
  import FeatureOutlet from './lib/FeatureOutlet.svelte';
  import {
    allowedRoute,
    readRoute,
    routeHref,
    sectionRoute,
    homeRoute,
    type AppRoute,
  } from './lib/navigation';
  import { captureSession, invalidateSession } from './lib/session';
  import Sidebar from './lib/ui/Sidebar.svelte';
  import NavigationDrawer from './lib/ui/NavigationDrawer.svelte';
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
  let route = $state<AppRoute>({ ...homeRoute });
  const section = $derived(route.section);
  const settingsSection = $derived(route.settings);
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
  import DesktopVersion from './lib/DesktopVersion.svelte';
  import WebUpdateReload from './lib/WebUpdateReload.svelte';
  import { connectDesktopUpdates } from './lib/desktop-updates';
  import { connectTools } from './lib/providers/tools-events';
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
  let mediaRevision = $state(0);
  import { Menu } from '@lucide/svelte';
  import {
    api,
    ApiError,
    Events,
    type User,
    desktop,
    initializeTransport,
    changeServer,
    serverUrl,
  } from './lib/api';
  let loading = $state(true),
    setup = $state(false),
    user = $state<User | null>(null),
    error = $state(''),
    connected = $state(false);
  let authOptions = $state<AuthOptions | null>(null);
  let recoveryToken = $state(
    new URL(location.href).hash.startsWith('#recovery=')
      ? new URL(location.href).hash.slice(10)
      : '',
  );
  let desktopRequest = $state(
    new URLSearchParams(location.search).get('desktop') ?? '',
  );
  const signInError =
    new URLSearchParams(location.search).get('auth_error') ?? '';
  const providerPage = $derived(
    ['YouTube', 'Twitch', 'Kick'].includes(section),
  );
  let preferencesRevision = $state(0);
  let unsavedChanges = $state(false);
  let timeFormat = $state<'12h' | '24h'>('24h');
  let events: Events | undefined;
  let navigationReady = $state(false);
  let settingsRevision = $state(0);
  function restoreNavigation() {
    if (!user) return;
    let saved: AppRoute = { ...homeRoute };
    try {
      const value = JSON.parse(
        localStorage.getItem(
          `thelxinoe:${serverUrl()}:${user.id}:navigation`,
        ) ?? 'null',
      );
      if (value)
        saved = sectionRoute(
          value.section,
          value.settingsSection === 'updates'
            ? desktop
              ? 'connection'
              : user.role === 'admin'
                ? 'server'
                : 'account'
            : value.settingsSection,
        );
    } catch {
      /* Ignore obsolete device navigation. */
    }
    route = allowedRoute(
      readRoute(new URL(location.href), saved),
      user,
      desktop,
      $providers,
    );
    writeLocation(route, true);
    navigationReady = true;
  }
  $effect(() => {
    if (!navigationReady || !user) return;
    localStorage.setItem(
      `thelxinoe:${serverUrl()}:${user.id}:navigation`,
      JSON.stringify({ section, settingsSection }),
    );
    const allowed = allowedRoute(route, user, desktop, $providers);
    if (routeHref(allowed) !== routeHref(route)) go(allowed, true);
    document.title = `${section} · Thelxinoe`;
  });
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
    error = '';
    try {
      await fn();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
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
    if (!setup)
      authOptions = await api<AuthOptions>('/auth/methods').catch(() => null);
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
    const ownsSession = captureSession();
    const value = await api<{
      timezone: string;
      time_format: '12h' | '24h';
    }>('/me/preferences');
    if (!ownsSession()) return;
    timeFormat = value.time_format;
    if (user) user = { ...user, timezone: value.timezone };
  }
  function startEvents() {
    const ownsSession = captureSession();
    events?.close();
    events = new Events(
      (event) => {
        if (!ownsSession()) return;
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
          settingsRevision++;
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
            settingsRevision++;
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
        if (event.kind === 'jobs.changed') settingsRevision++;
      },
      (value) => {
        if (!ownsSession()) return;
        connected = value;
        if (value && user) void refreshAppearance(user.id);
      },
    );
    events.connect();
  }
  async function prepareSignIn(address: string) {
    teardownSession();
    if (desktop) {
      await changeServer(address);
      serverAddress = serverUrl();
    }
    return checkServer();
  }
  async function signedIn(current: User) {
    user = current;
    setup = false;
    if (returnToService()) return;
    await Promise.all([
      loadAppearance(user.id),
      loadDisplayPreferences(),
      loadProviders(),
    ]);
    restoreNavigation();
    startEvents();
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
      teardownSession();
      await api('/auth/logout', 'POST');
    });
  }
  function teardownSession() {
    cancelVerification();
    invalidateSession();
    navigationReady = false;
    playbackRequest++;
    resetAppearance();
    playing = null;
    user = null;
    events?.close();
    events = undefined;
    connected = false;
    unsavedChanges = false;
    mobileNavOpen = false;
    route = { ...homeRoute };
    timeFormat = '24h';
  }
  function writeLocation(destination: AppRoute, replace: boolean, scroll = 0) {
    const url = new SvelteURL(location.href);
    url.searchParams.delete('section');
    url.searchParams.delete('youtube_link');
    url.hash = routeHref(destination);
    history[replace ? 'replaceState' : 'pushState'](
      { thelxinoeRoute: !replace || history.state?.thelxinoeRoute, scroll },
      '',
      url,
    );
  }
  function go(
    destination: AppRoute,
    replace = false,
    preserveWorkspace = false,
  ) {
    if (!user) return;
    const next = allowedRoute(destination, user, desktop, $providers);
    if (routeHref(next) === routeHref(route)) return;
    const scroll = workspace?.scrollTop ?? 0;
    history.replaceState({ ...history.state, scroll }, '', location.href);
    route = next;
    writeLocation(next, replace, preserveWorkspace ? scroll : 0);
    if (!preserveWorkspace) {
      mobileNavOpen = false;
      unsavedChanges = false;
      error = '';
      void restoreWorkspace(0, true);
    }
  }
  function updateFilters(destination: AppRoute, replace = true) {
    go(destination, replace, true);
  }
  async function restoreWorkspace(scroll: number, focus = false) {
    const destination = route;
    await preloadPage(destination).catch(() => {});
    if (routeHref(destination) !== routeHref(route)) return;
    await tick();
    if (workspace) workspace.scrollTop = scroll;
    if (focus) {
      const heading = main?.querySelector<HTMLElement>('header h1');
      if (heading) {
        heading.tabIndex = -1;
        heading.focus({ preventScroll: true });
      }
    }
  }
  async function navigate(name: string) {
    mobileNavOpen = false;
    go(sectionRoute(name, settingsSection));
  }
  function openSettings(name: string, service?: string) {
    go({ ...sectionRoute('Settings', name), service });
  }
  function back() {
    if (history.state?.thelxinoeRoute) history.back();
    else
      go(
        {
          ...route,
          detail: undefined,
          item: undefined,
          playlist: undefined,
          requests: false,
        },
        true,
      );
  }
  async function switchServer(address: string) {
    teardownSession();
    await changeServer(address);
    await boot();
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
      openSettings('account');
    if (playing && !providerEnabled(playing.id)) {
      playbackRequest++;
      if (desktop)
        void invoke('mpv_command', { command: 'stop', value: null }).catch(
          (e) => (error = String(e)),
        );
      playing = null;
    }
  });
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
    const locationChanged = () => {
      if (location.hash.startsWith('#recovery=')) {
        teardownSession();
        recoveryToken = location.hash.slice(10);
        return;
      }
      if (!user) return;
      route = allowedRoute(
        readRoute(new URL(location.href), route),
        user,
        desktop,
        $providers,
      );
      mobileNavOpen = false;
      void restoreWorkspace(history.state?.scroll ?? 0, true);
    };
    const sessionExpired = () => teardownSession();
    const cancelledEdits = () => {
      unsavedChanges = true;
    };
    window.addEventListener('popstate', locationChanged);
    window.addEventListener('hashchange', locationChanged);
    window.addEventListener('thelxinoe-session-expired', sessionExpired);
    window.addEventListener('thelxinoe-unsaved-changes', cancelledEdits);
    const settingsRequested = (event: Event) => {
      if (
        (event as CustomEvent<string>).detail === 'server' &&
        user?.role === 'admin'
      ) {
        openSettings('server');
      }
    };
    window.addEventListener('thelxinoe-open-settings', settingsRequested);
    window.addEventListener('thelxinoe-update-required', incompatible);
    void boot().then(async () => {
      if (!desktop) return;
      if (user) await preloadPage(route).catch(() => {});
      if (playing) await players.native().catch(() => {});
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
      window.removeEventListener('thelxinoe-open-settings', settingsRequested);
      window.removeEventListener('popstate', locationChanged);
      window.removeEventListener('hashchange', locationChanged);
      window.removeEventListener('thelxinoe-session-expired', sessionExpired);
      window.removeEventListener('thelxinoe-unsaved-changes', cancelledEdits);
      teardownSession();
      sidebarMotion.destroy();
      media.removeEventListener('change', resize);
    };
  });
</script>

<svelte:document onclick={playYoutubeLink} />

{#if desktop}<WindowTitlebar />{/if}
<AuthVerification />
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
{:else if recoveryToken}
  <AuthLayout
    ><Recovery
      token={recoveryToken}
      completed={() => {
        recoveryToken = '';
        void boot();
      }}
    /></AuthLayout
  >
{:else if !user}
  <AuthLayout
    ><SignIn
      {setup}
      bind:serverAddress
      options={authOptions}
      prepare={prepareSignIn}
      authenticated={signedIn}
      initialError={signInError || error}
    /></AuthLayout
  >
{:else if !desktop && desktopRequest}
  <AuthLayout
    ><DesktopApproval
      request={desktopRequest}
      {user}
      completed={() => {
        desktopRequest = '';
        history.replaceState(null, '', location.pathname);
      }}
    /></AuthLayout
  >
{:else}
  <div
    class="app-shell flex h-dvh overflow-hidden desktop-shell:mt-8 desktop-shell:h-[calc(100dvh-32px)]"
    bind:this={shell}
  >
    {#snippet navigation(currentUser: User)}
      <Sidebar
        {section}
        {settingsSection}
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
        {#if unsavedChanges}<Notice role="status"
            >Unsaved changes were cancelled when leaving the form.</Notice
          >{/if}
        {#if playing && desktop}{#await players.native()}<p role="status">
              Loading player…
            </p>{:then { default: NativePlayer }}<NativePlayer
              choice={playing}
              closed={(closedChoice) => {
                if (playing === closedChoice) playing = null;
              }}
            />{:catch error}<Notice role="alert" variant="error"
              >{String(error)}</Notice
            >{/await}{:else if playing && playing.kind === 'track'}{#await players.music()}<p
              role="status"
            >
              Loading player…
            </p>{:then { default: MusicPlayer }}<MusicPlayer
              choice={playing}
              closed={() => (playing = null)}
            />{:catch error}<Notice role="alert" variant="error"
              >{String(error)}</Notice
            >{/await}{:else if playing}<div
            class={[
              'player-frame',
              providerPage
                ? '-mt-3 -ml-3'
                : section === 'Settings'
                  ? ''
                  : '-mx-6 -mt-6 narrow:-mx-4 narrow:-mt-4 compact:-mx-3 compact:-mt-3',
            ]}
          >
            {#await players.video()}<p role="status">
                Loading player…
              </p>{:then { default: Player }}<Player
                choice={playing}
                closed={() => (playing = null)}
                resizable
              />{:catch error}<Notice role="alert" variant="error"
                >{String(error)}</Notice
              >{/await}
          </div>{/if}
        <FeatureOutlet
          {route}
          {user}
          {timeFormat}
          {mediaRevision}
          {catalogRevision}
          {accountRevision}
          {preferencesRevision}
          {settingsRevision}
          {scans}
          {playing}
          play={playMedia}
          navigate={(name) => void navigate(name)}
          {openSettings}
          {go}
          {updateFilters}
          {back}
          sessionEnded={teardownSession}
          {switchServer}
          displayChanged={(zone, format) => {
            timeFormat = format;
            if (user) user = { ...user, timezone: zone };
          }}
          profileChanged={(id, avatar) => {
            if (user?.id === id) user = { ...user, avatar };
          }}
        />
      </div>
    </main>
  </div>
{/if}
