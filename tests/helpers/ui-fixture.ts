import type { Page, WebSocketRoute } from '@playwright/test';
import { current } from './versions';
import { serviceView } from '../../scripts/mock/service-view.mjs';
import type { RecyclarrConfiguration } from '../../frontend/src/lib/services/recyclarr-configuration';

// Delivers server events to the page; install it before the page opens.
export async function installServerEvents(page: Page) {
  let socket: WebSocketRoute | undefined;
  let id = 0;
  await page.routeWebSocket(/\/api\/v1\/events(?:\?|$)/, (route) => {
    socket = route;
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({
      json: { ticket: 'layout', cursor: 0, epoch: 'layout', version: current },
    }),
  );
  return {
    connected: () => socket !== undefined,
    send(kind: string, payload: unknown = {}) {
      socket!.send(JSON.stringify({ id: ++id, kind, payload }));
    },
  };
}

// All requests stay in the browser. This fixture never uses a server account.
export async function installUiFixture(
  page: Page,
  options: {
    role?: 'admin' | 'user';
    section?: string;
    preserveNavigation?: boolean;
    settingsSection?: string;
    signedIn?: boolean;
    approvalUsers?: { id: string; username: string; enabled: boolean }[];
    endpointFailures?: Record<string, string>;
    stackProvisionState?: string;
    stackCapabilities?: {
      can_retire: boolean;
      can_remove: boolean;
      can_recreate: boolean;
    };
    managedNzbget?: boolean;
    managerKind?: 'radarr' | 'sonarr' | 'lidarr';
    managerDefaultsUnset?: boolean;
    managerLibraryError?: string | null;
    serviceUpdateState?: string;
    /** Registered services that stay Connected instead of Owned. */
    connected?: string[];
    /** Kinds a test mocks itself that Thelxinoe owns. */
    owned?: string[];
  } = {},
) {
  const managerKind = options.managerKind ?? 'radarr';
  const managerName = managerKind[0].toUpperCase() + managerKind.slice(1);
  const user = {
    id: 'layout-fixture',
    username: 'Layout viewer',
    role: options.role ?? 'user',
    timezone: 'UTC',
  };
  let signedIn = options.signedIn ?? true;
  let preferences = {
    timezone: 'UTC',
    timezone_override: null as string | null,
    server_timezone: 'UTC',
    time_format: '24h' as '12h' | '24h',
    time_format_override: null as '12h' | '24h' | null,
    server_time_format: '24h' as '12h' | '24h',
  };
  let serverSettings = {
    timezone: 'UTC',
    time_format: '24h' as '12h' | '24h',
    video_transcoding: true,
    audio_transcoding: true,
  };
  let productUpdate = {
    version: current,
    timezone: 'UTC',
    configured: false,
    policy: { policy: 'automatic', window_start: 3, window_end: 5 },
    release: null,
    observation: null,
    controller: { items: [] },
  };
  let appearance = {
    provider_preferences: {},
    audio_volume: 1,
    player_height: null as number | null,
    youtube_card_shortcuts: [],
    theme: 'light',
    sidebar_collapsed: false,
    poster_columns: 6,
    video_columns: 6,
    fade_watched: true,
    thumbnail_fit: 'contain',
  };
  // Installs keep a setup hold until their reviewed setup finishes.
  const stackProvisions = () => {
    const radarr = options.stackProvisionState ?? 'complete';
    const nzbget = options.stackProvisionState ?? 'queued';
    return [
      {
        id: 'managed-radarr',
        kind: 'radarr',
        state: radarr,
        host_port: 17878,
        container_id: 'container-radarr',
        service_id: 'manager-radarr',
        error: null,
        native_url: '',
        origin: 'installed',
        hold: radarr === 'complete' ? null : 'setup',
        transfer_id: null,
      },
      ...(options.managedNzbget
        ? [
            {
              id: 'managed-nzbget',
              kind: 'nzbget',
              state: nzbget,
              host_port: 16789,
              container_id: null,
              service_id: null,
              error: null,
              native_url: '',
              origin: 'installed',
              hold: nzbget === 'complete' ? null : 'setup',
              transfer_id: null,
            },
          ]
        : []),
    ];
  };
  const owned = new Set(options.owned ?? []);
  // Kinds a test has removed from its own service mocks.
  const cleared = new Set<string>();
  // Active transfer operations a test reports for a service kind.
  const operations = new Map<string, Record<string, unknown>>();
  // The server composes its services view from these parts. Reading them
  // through the page lets a test replace one part as it would that endpoint.
  const viewParts = {
    managers: '/admin/managers',
    support: '/admin/support',
    containers: '/admin/managers/containers',
    stack: '/admin/stack',
    connections: '/admin/service-connections',
    updates: '/admin/service-updates',
  };
  // Undefined once the page has closed or navigated away from the read.
  const composeView = async (stages: unknown) => {
    const parts = await page
      .evaluate(
        async (paths) =>
          Object.fromEntries(
            await Promise.all(
              Object.entries(paths).map(async ([name, path]) => {
                const response = await fetch(`/api/v1${path}`);
                const body = await response.json();
                return [
                  name,
                  response.ok
                    ? { value: body }
                    : { error: body.error?.message ?? String(response.status) },
                ];
              }),
            ),
          ),
        viewParts,
      )
      .catch(() => undefined);
    return parts && serviceView({ ...parts, stages: { value: stages } });
  };
  const writes: { path: string; method: string; body: unknown }[] = [];
  const errors: string[] = [];
  const unexpected: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(
    ({ userId, section, settingsSection, preserveNavigation }) => {
      const key = `thelxinoe::${userId}:navigation`;
      if (preserveNavigation && localStorage.getItem(key)) return;
      localStorage.setItem(key, JSON.stringify({ section, settingsSection }));
    },
    {
      userId: user.id,
      section: options.section ?? 'Settings',
      preserveNavigation: options.preserveNavigation ?? false,
      settingsSection: options.settingsSection ?? 'account',
    },
  );
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.slice('/api/v1'.length);
    const method = request.method();
    const body = request.postDataJSON();
    if (!['GET', 'HEAD'].includes(method) && path !== '/auth/event-ticket')
      writes.push({ path, method, body });
    const json = (value: unknown) => route.fulfill({ json: value });
    if (path === '/admin/tools') return json({ items: [], supported: false });
    if (path === '/health') return json({ api_version: 1 });
    if (path === '/online/providers')
      return json({ youtube: true, twitch: true, kick: true });
    if (path === '/seerr/status') return json({ configured: false });
    if (path === '/setup') return json({ setup_required: false });
    if (path === '/auth/methods')
      return json({
        canonical_url: new URL(route.request().url()).origin,
        passkeys: false,
        oidc: null,
        server_id: 'ui-fixture',
      });
    if (path === '/me/auth')
      return json({
        password: true,
        totp: false,
        passkeys: [],
        oidc: null,
        fresh: true,
      });
    if (path === '/auth/devices' || path === '/me/auth/client-passwords')
      return json({ items: [] });
    if (path === '/admin/auth/oidc')
      return json({ provider: null, redirect_uri: null });
    if (path === '/auth/login') {
      signedIn = true;
      return json({ user });
    }
    if (path === '/auth/me')
      return signedIn
        ? json({ user })
        : route.fulfill({
            status: 401,
            json: { error: { message: 'Sign in required' } },
          });
    if (path === '/auth/event-ticket')
      return route.fulfill({
        status: 503,
        json: { error: { message: 'Events disabled in fixture' } },
      });
    if (method === 'GET' && options.endpointFailures?.[path])
      return route.fulfill({
        status: 503,
        json: {
          error: {
            code: 'unavailable',
            message: options.endpointFailures[path],
          },
        },
      });
    if (path === '/me/home')
      return json({
        continue_watching: [],
        next_up: [],
        watch_later: [],
        favorites: [],
      });
    if (['/online/youtube', '/online/twitch'].includes(path))
      return json({ configured: false, account: { status: 'disconnected' } });
    // Home reads these beside the account before knowing it is disconnected.
    if (path === '/online/youtube/home')
      return json({ continue_watching: [], next_up: [] });
    if (path === '/online/twitch/feed') return json({ items: [] });
    if (path === '/online/kick')
      return json({ connected: false, configured: false, items: [] });
    if (path === '/me/appearance') {
      if (method === 'PATCH') appearance = { ...appearance, ...body };
      return json(appearance);
    }
    if (path === '/me/preferences') {
      if (method === 'PUT') {
        const nextPreferences = {
          ...preferences,
          timezone_override: body.timezone ?? null,
          timezone: body.timezone ?? preferences.server_timezone,
        };
        if (Object.prototype.hasOwnProperty.call(body, 'time_format')) {
          nextPreferences.time_format_override = body.time_format;
          nextPreferences.time_format =
            body.time_format ?? preferences.server_time_format;
        }
        preferences = nextPreferences;
      }
      return json(preferences);
    }
    if (path === '/timezones')
      return json({ timezones: ['UTC', 'Europe/Paris', 'America/New_York'] });
    if (path === '/auth/sessions')
      return json({
        current: 'session-0',
        items: Array.from({ length: 40 }, (_, index) => ({
          id: `session-${index}`,
          name: `Browser ${index}`,
          transport: 'web',
          last_seen: 1789984800,
        })),
      });
    if (path === '/users') return json({ items: [user] });
    if (path === '/admin/jobs')
      return json({
        items: Array.from({ length: 40 }, (_, index) => ({
          id: `job-${index}`,
          kind: `Fixture job ${index}`,
          state: 'complete',
          error: null,
        })),
      });
    if (path === '/admin/health')
      return json({
        version: current,
        controller: true,
        cache_free_bytes: 1024 ** 3,
      });
    if (path === '/admin/operations')
      return json({
        storage: {
          state: { bytes: 0, free_bytes: 1024 ** 3, partial: false },
          cache: { bytes: 0, free_bytes: 1024 ** 3, partial: false },
        },
        support: { items: [] },
        playback: [],
        errors: [],
        services: [],
      });
    if (path === '/admin/devices') return json({ items: [] });
    if (path === '/admin/settings') {
      if (method === 'PUT') {
        serverSettings = { ...serverSettings, ...body };
        preferences.server_timezone = serverSettings.timezone;
        preferences.server_time_format = serverSettings.time_format;
        if (preferences.timezone_override === null)
          preferences.timezone = serverSettings.timezone;
        if (preferences.time_format_override === null)
          preferences.time_format = serverSettings.time_format;
        productUpdate.timezone = serverSettings.timezone;
      }
      return json(serverSettings);
    }
    if (path === '/admin/product-update') return json(productUpdate);
    if (path === '/admin/product-update/policy') {
      if (method === 'POST')
        productUpdate = {
          ...productUpdate,
          policy: body,
        };
      return json({ saved: true });
    }
    if (path === '/admin/acquisition/users')
      return json({
        items: options.approvalUsers ?? [],
      });
    if (path.startsWith('/admin/acquisition/users/') && method === 'PUT') {
      const entry = (options.approvalUsers ?? []).find(
        (candidate) => candidate.id === path.split('/').at(-1),
      );
      if (entry) entry.enabled = Boolean(body.enabled);
      return json({ saved: true });
    }
    if (path === '/admin/service-connections') return json({ items: [] });
    if (path === '/admin/managers')
      return json({
        items: [
          {
            id: `manager-${managerKind}`,
            name: managerName,
            kind: managerKind,
            container_id: `container-${managerKind}`,
            port: { radarr: 7878, sonarr: 8989, lidarr: 8686 }[managerKind],
            url_base: `/services/${managerKind}`,
            access_url: `/services/${managerKind}`,
            version: '6.0.0',
            defaults: {
              root_folder: options.managerDefaultsUnset ? '' : '/media/movies',
              quality_profile: options.managerDefaultsUnset ? 0 : 1,
              metadata_profile: null,
              monitored: true,
            },
            checked_at: 1789984800,
            library_error: options.managerLibraryError ?? null,
            error: null,
          },
        ],
      });
    if (path === '/admin/support')
      return json({
        items: [
          {
            id: 'support-prowlarr',
            name: 'Prowlarr',
            kind: 'prowlarr',
            container_id: 'container-prowlarr',
            port: 9696,
            version: '2.0.0',
            native_url: '',
            url_base: '/services/prowlarr',
            access_url: '/services/prowlarr',
            checked_at: 1789984800,
            error: null,
          },
          {
            id: 'support-nzbget',
            name: 'NZBGet',
            kind: 'nzbget',
            container_id: 'container-nzbget',
            port: 6789,
            version: '25.0',
            native_url: '',
            url_base: '',
            access_url: '/services/nzbget',
            checked_at: 1789984800,
            error: null,
          },
        ],
      });
    if (
      path === `/admin/managers/manager-${managerKind}/defaults` &&
      method === 'PUT'
    )
      return json({ saved: true });
    if (path === `/admin/managers/manager-${managerKind}/options`)
      return json({
        roots: [{ id: 1, path: '/media/movies' }],
        profiles: [{ id: 1, name: 'HD-1080p' }],
        metadata_profiles: [],
      });
    if (path === '/admin/support/support-prowlarr')
      return json({
        health: [],
        indexers: [
          {
            id: 1,
            name: 'Fixture indexer',
            enabled: true,
            disabled_until: null,
          },
        ],
      });
    if (path === '/admin/support/support-nzbget/login' && method === 'POST')
      return json({ username: 'fixture', password: 'fixture-password' });
    if (path === '/admin/stack/managed-nzbget/login' && method === 'POST')
      return json({ username: 'thelxinoe', password: 'managed-password' });
    if (path === '/admin/support/support-nzbget')
      return json({
        paused: false,
        queue: [
          {
            id: 1,
            title: 'Active fixture',
            status: 'DOWNLOADING',
            size_mb: 8192,
            remaining_mb: 2048,
          },
          {
            id: 2,
            title: 'Paused fixture',
            status: 'PAUSED',
            size_mb: 2048,
            remaining_mb: 1024,
          },
          {
            id: 3,
            title: 'Unpacking fixture',
            status: 'UNPACKING',
            size_mb: 4096,
            remaining_mb: 0,
          },
        ],
        history: Array.from({ length: 30 }, (_, i) => ({
          id: 100 + i,
          title: `Finished fixture ${i + 1}`,
          status:
            i === 0
              ? 'FAILURE/UNPACK'
              : i === 1
                ? 'SUCCESS/HIDDEN'
                : 'SUCCESS/ALL',
          size_mb: i === 1 ? null : 2048,
          remaining_mb: null,
          downloaded_mb: i === 1 ? null : 2048,
          history_time: 1789984800 - i,
        })),
      });
    if (path === '/admin/stack/managed-radarr/action')
      return json({ queued: true });
    if (path === '/admin/services') {
      const registered: Record<string, { id: string; version: string }> = {
        [managerKind]: { id: `manager-${managerKind}`, version: '6.0.0' },
        prowlarr: { id: 'support-prowlarr', version: '2.0.0' },
        nzbget: { id: 'support-nzbget', version: '25.0' },
      };
      const provisions = stackProvisions();
      const stages = {
        items: [
          'seerr',
          'recyclarr',
          'radarr',
          'sonarr',
          'lidarr',
          'bazarr',
          'prowlarr',
          'nzbget',
        ].map((kind) => {
          const service = cleared.has(kind) ? undefined : registered[kind];
          const provision = cleared.has(kind)
            ? undefined
            : provisions.find((item) => item.kind === kind);
          const connected = !!service && options.connected?.includes(kind);
          const stage = connected
            ? 'connected'
            : service || provision || (owned.has(kind) && !cleared.has(kind))
              ? 'owned'
              : 'none';
          const hold = stage === 'owned' ? (provision?.hold ?? null) : null;
          return {
            kind,
            stage,
            hold,
            agreed: true,
            draft_review: null,
            actions: [],
            connection: service
              ? {
                  id: service.id,
                  container_id: `container-${kind}`.padEnd(64, '0'),
                  container_name: kind,
                  engine: 'fixture-engine',
                  version: service.version,
                  verified: true,
                  operational: stage === 'owned' && !hold,
                  checked_at: 1789984800,
                  error: null,
                }
              : null,
            ownership:
              stage === 'owned'
                ? {
                    id: provision?.id ?? `transferred-${kind}`,
                    state: provision?.state ?? 'complete',
                    origin: provision ? 'installed' : 'transferred',
                    hold,
                    transfer_id: null,
                  }
                : null,
            operation: operations.get(kind) ?? null,
          };
        }),
      };
      const view = await composeView(stages);
      return view ? json(view) : route.abort().catch(() => {});
    }
    if (path === '/admin/managers/containers')
      return json({
        items: [
          {
            id: 'container-radarr',
            names: ['/radarr'],
            image: 'ghcr.io/example/radarr:stable',
            state: 'running',
            ports: [
              {
                Type: 'tcp',
                PrivatePort: 7878,
                PublicPort: 17878,
                IP: '127.0.0.1',
              },
            ],
          },
          {
            id: 'container-prowlarr',
            names: ['/prowlarr'],
            image: 'linuxserver/prowlarr:stable',
            state: 'running',
            ports: [],
          },
          {
            id: 'container-sonarr',
            names: ['sonarr'],
            image: 'linuxserver/sonarr',
            state: 'running',
          },
        ],
      });
    if (path === '/admin/stack')
      return json({
        items: [
          {
            id: 'managed-radarr',
            kind: 'radarr',
            name: 'Managed Radarr',
            phase: 'active',
            image: 'ghcr.io/example/radarr:stable',
            drift: false,
            running: true,
            existence: 'present',
            status: 'running',
            can_recreate: false,
            can_retire: false,
            error: null,
            transfer_pending: false,
            ...options.stackCapabilities,
          },
          {
            id: 'controller-bazarr',
            kind: 'bazarr',
            name: 'Managed Bazarr',
            phase: 'active',
            image: 'ghcr.io/example/bazarr:stable',
            drift: false,
            running: true,
            existence: 'present',
            status: 'running',
            can_recreate: false,
            can_retire: false,
            error: null,
            transfer_pending: false,
          },
        ],
        provisions: stackProvisions(),
      });
    if (path === '/admin/recyclarr/configuration') {
      const defaults = {
        image: 'recyclarr/recyclarr:fixture',
        resources: 'fixture-resources',
        files: {
          'recyclarr.yml': '# No connected instances.\n',
          'settings.yml': '{}\n',
        },
      };
      return json({
        revision: 'fixture-configuration',
        mode: 'defaults',
        image: defaults.image,
        files: defaults.files,
        defaults,
        base_defaults: defaults,
        bindings: [],
        candidate: null,
      } satisfies RecyclarrConfiguration);
    }
    if (path === '/admin/service-updates')
      return json({
        policies: [
          {
            service_id: 'managed-radarr',
            policy: 'inherit',
            window_start: 0,
            window_end: 0,
            candidate: 'ghcr.io/example/radarr:new',
            error: null,
          },
        ],
        timezone: 'UTC',
        items: options.serviceUpdateState
          ? [
              {
                id: 'update-radarr',
                service_id: 'managed-radarr',
                state: options.serviceUpdateState,
                classification:
                  options.serviceUpdateState === 'blocked'
                    ? 'incompatible'
                    : null,
                error:
                  options.serviceUpdateState === 'blocked'
                    ? 'Candidate has an incompatible database'
                    : null,
              },
            ]
          : [],
        services: [{ id: 'managed-radarr', kind: 'radarr' }],
        server_policy: {
          policy: productUpdate.policy.policy,
          window_start: productUpdate.policy.window_start,
          window_end: productUpdate.policy.window_end,
        },
      });
    if (path === '/admin/service-updates/policy/managed-radarr')
      return json({ saved: true });
    if (path === '/playback/preferences')
      return json({
        quality: 'auto',
        audio_language: 'eng',
        subtitle_language: 'eng',
        subtitles: false,
        replay_gain: 'track',
      });
    if (path === '/me/segments')
      return json({
        Intro: 'Ask',
        Recap: 'Ask',
        Credits: 'Ask',
        Preview: 'Ask',
      });
    if (path === '/catalog')
      return json({
        items: Array.from({ length: 12 }, (_, index) => ({
          id: `movie-${index}`,
          kind: 'movie',
          title: `Fixture movie ${index}`,
          year: 2026,
          available: true,
          metadata: {},
        })),
      });
    if (path === '/acquisition/requests')
      return json({ items: [], page: 1, pages: 1 });
    if (path === '/catalog/roots')
      return json({ items: [], media_mount: '/media' });
    if (path === '/catalog/lookup')
      return json({ media_id: null, title: null, status: null });
    if (['/catalog/collections', '/me/attention'].includes(path))
      return json({ items: [] });
    if (path === '/requests/capabilities')
      return json({ movies: false, tv: false, music: false });
    unexpected.push(`${method} ${path}`);
    return route.fulfill({
      status: 404,
      json: { error: { message: `Unexpected fixture request: ${path}` } },
    });
  });
  return {
    writes,
    errors,
    unexpected,
    owned,
    cleared,
    operations,
    // Quick successive changes save once input pauses; tests that reload
    // right after them wait for the save.
    savedAppearance: () => appearance,
  };
}
