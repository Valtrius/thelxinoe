import type { Page } from '@playwright/test';

// All requests stay in the browser. This fixture never uses a server account.
export async function installUiFixture(
  page: Page,
  options: {
    role?: 'admin' | 'user';
    section?: string;
    settingsSection?: string;
    signedIn?: boolean;
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
    if (path === '/me/appearance') {
      if (method === 'PATCH') appearance = { ...appearance, ...body };
      return json(appearance);
    }
    if (path === '/me/preferences') {
      if (method === 'PUT')
        preferences = {
          ...preferences,
          timezone_override: body.timezone,
          timezone: body.timezone ?? 'UTC',
          time_format: body.time_format ?? preferences.time_format,
        };
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
    if (path === '/admin/settings') return json({ timezone: 'UTC' });
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
