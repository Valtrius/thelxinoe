import type { Page } from '@playwright/test';

// All requests stay in the browser. This fixture never uses a server account.
export async function installUiFixture(
  page: Page,
  options: {
    role?: 'admin' | 'user';
    section?: string;
    settingsSection?: string;
    signedIn?: boolean;
    approvalUsers?: { id: string; username: string; enabled: boolean }[];
    endpointFailures?: Record<string, string>;
    stackProvisionState?: string;
  } = {},
) {
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
  };
  let productUpdate = {
    version: '0.1.0',
    timezone: 'UTC',
    configured: false,
    policy: { policy: 'notify', window_start: 3, window_end: 5 },
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
    card_columns: 6,
    fade_watched: true,
    thumbnail_fit: 'contain',
  };
  const writes: { path: string; method: string; body: unknown }[] = [];
  const errors: string[] = [];
  const unexpected: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(
    ({ userId, section, settingsSection }) => {
      localStorage.setItem(
        `thelxinoe::${userId}:navigation`,
        JSON.stringify({ section, settingsSection }),
      );
    },
    {
      userId: user.id,
      section: options.section ?? 'Settings',
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
    if (path === '/health') return json({ api_version: 1 });
    if (path === '/setup') return json({ setup_required: false });
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
          state: 'completed',
          error: null,
        })),
      });
    if (path === '/admin/health')
      return json({
        version: '0.1.0',
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
    if (path === '/admin/managers')
      return json({
        items: [
          {
            id: 'manager-radarr',
            name: 'Radarr',
            kind: 'radarr',
            container_id: 'container-radarr',
            port: 7878,
            version: '6.0.0',
            defaults: {
              root_folder: '/media/movies',
              quality_profile: 1,
              metadata_profile: null,
              monitored: true,
            },
            checked_at: 1789984800,
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
            version: '2.0.0',
            native_url: 'https://prowlarr.example.test',
            checked_at: 1789984800,
            error: null,
          },
        ],
      });
    if (path === '/admin/managers/containers')
      return json({
        items: [
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
            error: null,
            transfer_pending: false,
          },
          {
            id: 'controller-bazarr',
            kind: 'bazarr',
            name: 'Managed Bazarr',
            phase: 'active',
            image: 'ghcr.io/example/bazarr:stable',
            drift: false,
            running: true,
            error: null,
            transfer_pending: false,
          },
        ],
        provisions: [
          {
            id: 'managed-radarr',
            kind: 'radarr',
            state: options.stackProvisionState ?? 'complete',
            host_port: 17878,
            container_id: 'container-radarr',
            service_id: 'manager-radarr',
            error: null,
            native_url: '',
            origin: 'installed',
          },
        ],
      });
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
        items: [],
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
    if (['/catalog/collections', '/me/notifications'].includes(path))
      return json({ items: [] });
    if (path === '/requests/capabilities')
      return json({ movies: false, tv: false, music: false });
    unexpected.push(`${method} ${path}`);
    return route.fulfill({
      status: 404,
      json: { error: { message: `Unexpected fixture request: ${path}` } },
    });
  });
  return { writes, errors, unexpected };
}
