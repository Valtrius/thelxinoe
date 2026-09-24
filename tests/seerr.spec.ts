import { expect, test, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { installUiFixture } from './helpers/ui-fixture';

const movie = {
  id: 11,
  mediaType: 'movie',
  title: 'Arrival',
  overview: 'A visitor arrives.',
  releaseDate: '2016-11-10',
  voteAverage: 8,
  relatedVideos: [
    {
      site: 'YouTube',
      key: 'xwdamnfS6IM',
      type: 'Trailer',
      name: 'Official Trailer',
    },
  ],
};
const show = {
  id: 22,
  mediaType: 'tv',
  name: 'Severance',
  overview: 'A divided workplace.',
  seasons: [
    { id: 1, seasonNumber: 1, name: 'Season 1', episodeCount: 9 },
    { id: 2, seasonNumber: 2, name: 'Season 2', episodeCount: 10 },
    { id: 3, seasonNumber: 3, name: 'Season 3', episodeCount: 10 },
  ],
  mediaInfo: { status: 4, seasons: [{ seasonNumber: 1, status: 5 }] },
};
const results = (items: unknown[]) => ({
  results: items,
  page: 1,
  totalPages: 1,
  totalResults: items.length,
});

async function discoverFixture(page: Page, role: 'admin' | 'user' = 'user') {
  const fixture = await installUiFixture(page, { section: 'Home', role });
  const requests: unknown[] = [];
  let requested = false;
  await page.route('**/api/v1/seerr/**', async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace('/api/v1/seerr', '');
    if (path === '/status')
      return route.fulfill({ json: { configured: true, ready: true } });
    if (path.startsWith('/discover/'))
      return route.fulfill({ json: results([movie, show]) });
    if (path === '/search')
      return route.fulfill({
        json: results(
          url.searchParams.get('q')?.includes('Sev') ? [show] : [movie],
        ),
      });
    if (path.endsWith('/recommendations'))
      return route.fulfill({ json: results([]) });
    if (path === '/movie/11')
      return route.fulfill({
        json: { ...movie, ...(requested ? { mediaInfo: { status: 2 } } : {}) },
      });
    if (path === '/tv/22') return route.fulfill({ json: show });
    if (path === '/requests' && route.request().method() === 'POST') {
      requests.push(route.request().postDataJSON());
      requested = true;
      return route.fulfill({ json: { id: 1, status: 1 } });
    }
    if (path === '/requests')
      return route.fulfill({
        json: {
          pageInfo: { pages: 1 },
          results: [
            {
              id: 1,
              status: 1,
              type: 'movie',
              media: { tmdbId: 11, mediaType: 'movie' },
              requestedBy: { username: 'Viewer' },
            },
          ],
        },
      });
    return route.fulfill({ status: 404, json: { error: { message: path } } });
  });
  return { ...fixture, requests };
}

test('posters align at the top of discovery rows and search rows with wrapped titles', async ({
  page,
}, testInfo) => {
  const fixture = await discoverFixture(page);
  const items = [
    movie,
    { ...show, name: 'A series with a title that spans two lines' },
  ];
  await page.route('**/api/v1/seerr/discover/**', (route) =>
    route.fulfill({ json: results(items) }),
  );
  await page.route('**/api/v1/seerr/search**', (route) =>
    route.fulfill({ json: results(items) }),
  );
  await page.goto('/');
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const search = page.getByRole('textbox', {
      name: 'Search movies and series',
    });
    for (const query of ['', 'title']) {
      await search.fill(query);
      const first = page
        .getByRole('button', { name: 'View Arrival', exact: true })
        .first();
      const second = page
        .getByRole('button', { name: `View ${items[1].name}`, exact: true })
        .first();
      await expect(second).toBeVisible();
      await page.mouse.move(0, 0);
      expect(
        (await second.locator('strong').boundingBox())!.height,
      ).toBeGreaterThan((await first.locator('strong').boundingBox())!.height);
      await expect
        .poll(async () =>
          Math.abs(
            (await first.locator(':scope > div').boundingBox())!.y -
              (await second.locator(':scope > div').boundingBox())!.y,
          ),
        )
        .toBeLessThanOrEqual(1);
      await testInfo.attach(`posters-${query ? 'search' : 'feed'}-${width}`, {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
    }
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('search opens dedicated details, requests once, and returns to the search', async ({
  page,
}) => {
  const fixture = await discoverFixture(page);
  await page.goto('/');
  await page
    .getByRole('textbox', { name: 'Search movies and series' })
    .fill('Arrival');
  await page.getByRole('button', { name: 'View Arrival', exact: true }).click();
  await expect(page).toHaveURL(/#discover\/movie\/11$/);
  await expect(
    page.getByRole('heading', { name: 'Arrival', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Request movie', exact: true })
    .click();
  await expect(page.getByText('Request sent for approval.')).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Pending approval', exact: true }),
  ).toBeDisabled();
  expect(fixture.requests).toEqual([
    { media_type: 'movie', media_id: 11, seasons: [] },
  ]);
  await page.getByRole('button', { name: 'Back to discover' }).click();
  await expect(
    page.getByRole('textbox', { name: 'Search movies and series' }),
  ).toHaveValue('Arrival');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('a trailer resolves an uncached video and plays in the shared player without leaving details', async ({
  page,
}) => {
  const fixture = await discoverFixture(page);
  const calls: string[] = [];
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route('**/api/v1/online/youtube/resolve', async (route) => {
    expect(route.request().postDataJSON()).toEqual({ video_id: 'xwdamnfS6IM' });
    calls.push('resolve');
    await held;
    await route.fulfill({ json: { ready: true } });
  });
  await page.route('**/api/v1/catalog/youtube:*/playback', async (route) => {
    calls.push('metadata');
    await route.fulfill({
      json: {
        sources: [],
        progress: [],
        watched: false,
        preferences: { quality: 'auto', subtitles: false },
      },
    });
  });
  await page.route('**/api/v1/playback', async (route) => {
    const input = route.request().postDataJSON();
    expect(input.media_id).toBe('youtube:xwdamnfS6IM');
    calls.push('playback');
    await route.fulfill({
      json: {
        id: 'trailer',
        mode: 'direct',
        url: '/fixture/trailer.mp4',
        position: 0,
        duration: 10,
        timeline_start: 0,
        video: true,
        tracks: [],
        subtitles: [],
        selected_subtitle: 'off',
        options: input.options,
        probe: { streams: [{ codec_type: 'video' }] },
        replay_gain: 'off',
      },
    });
  });
  await page.route('**/api/v1/playback/trailer/progress', (route) =>
    route.fulfill({ json: { saved: true } }),
  );
  const video = execFileSync(
    'ffmpeg',
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-f',
      'lavfi',
      '-i',
      'color=c=0x243841:size=320x180:rate=12',
      '-t',
      '10',
      '-an',
      '-c:v',
      'libx264',
      '-preset',
      'ultrafast',
      '-pix_fmt',
      'yuv420p',
      '-movflags',
      'frag_keyframe+empty_moov',
      '-f',
      'mp4',
      'pipe:1',
    ],
    { windowsHide: true },
  );
  await page.route('**/fixture/trailer.mp4', (route) =>
    route.fulfill({ contentType: 'video/mp4', body: video }),
  );
  await page.goto('/#discover/movie/11');
  await page.getByRole('link', { name: 'Watch trailer' }).press('Enter');
  await expect.poll(() => calls).toEqual(['resolve']);
  release();
  await expect.poll(() => calls).toEqual(['resolve', 'metadata', 'playback']);
  const player = page.locator('video');
  await expect(player).toBeVisible();
  await expect
    .poll(() => player.evaluate((element) => element.currentTime))
    .toBeGreaterThan(0.2);
  expect(await player.evaluate((element) => element.videoWidth)).toBe(320);
  await expect(page).toHaveURL(/#discover\/movie\/11$/);
  expect(page.context().pages()).toHaveLength(1);
  await expect(
    page.getByRole('heading', { name: 'Arrival', exact: true }),
  ).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('desktop trailers resolve through the server and launch MPV with the same video choice', async ({
  page,
}) => {
  const fixture = await discoverFixture(page);
  const launches: unknown[] = [];
  let resolved = false;
  await page.route('**/api/v1/online/youtube/resolve', async (route) => {
    expect(route.request().postDataJSON()).toEqual({ video_id: 'xwdamnfS6IM' });
    resolved = true;
    await route.fulfill({ json: { ready: true } });
  });
  await page.exposeFunction('recordNativeLaunch', (choice: unknown) => {
    expect(resolved).toBe(true);
    launches.push(choice);
  });
  await page.addInitScript(() => {
    const bridge = window as unknown as {
      isTauri: boolean;
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
      recordNativeLaunch: (payload: unknown) => Promise<void>;
    };
    bridge.isTauri = true;
    bridge.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let callback = 0;
    bridge.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: 'main' },
        currentWebview: { label: 'main' },
      },
      transformCallback: () => ++callback,
      unregisterCallback() {},
      async invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'server_url') return location.origin;
        if (command === 'backend_request') {
          const response = await fetch(`/api/v1${args.path}`, {
            method: String(args.method),
            headers: { 'Content-Type': 'application/json' },
            ...(args.body ? { body: JSON.stringify(args.body) } : {}),
          });
          return { status: response.status, body: await response.json() };
        }
        if (command === 'mpv_state') return { status: 'stopped', media_id: '' };
        if (command === 'mpv_play') {
          await bridge.recordNativeLaunch(args);
          return {
            status: 'playing',
            media_id: 'youtube:xwdamnfS6IM',
            title: 'Arrival · Official Trailer',
          };
        }
        if (command === 'tools_get') return { tools: [], mpv: {} };
        if (command === 'plugin:app|version') return '0.1.0';
        if (command === 'plugin:event|listen') return ++callback;
        return false;
      },
    };
  });
  await page.goto('/#discover/movie/11');
  await page.getByRole('link', { name: 'Watch trailer' }).click();
  await expect
    .poll(() => launches)
    .toEqual([
      {
        choice: {
          id: 'youtube:xwdamnfS6IM',
          title: 'Arrival · Official Trailer',
          position: 0,
        },
        queue: null,
        music: false,
      },
    ]);
  await expect(page.locator('video')).toHaveCount(0);
  await expect(page).toHaveURL(/#discover\/movie\/11$/);
  expect(page.context().pages()).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('YouTube link lookup failures keep details open and allow another attempt', async ({
  page,
}) => {
  const fixture = await discoverFixture(page);
  const resolved: string[] = [];
  await page.route('**/api/v1/online/youtube/resolve', async (route) => {
    resolved.push(route.request().postDataJSON().video_id);
    await route.fulfill({
      status: 409,
      json: { error: { message: 'Video lookup is temporarily unavailable' } },
    });
  });
  await page.goto('/#discover/movie/11');
  const trailer = page.getByRole('link', { name: 'Watch trailer' });
  await trailer.click();
  await expect(page.getByRole('alert')).toContainText(
    'Video lookup is temporarily unavailable',
  );
  await expect(page).toHaveURL(/#discover\/movie\/11$/);
  await trailer.evaluate((link) =>
    link.setAttribute('href', 'https://youtu.be/9pkqztOC1WM'),
  );
  await trailer.click();
  await expect.poll(() => resolved).toEqual(['xwdamnfS6IM', '9pkqztOC1WM']);
  expect(page.context().pages()).toHaveLength(1);
  await expect(page.locator('video')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('discover uses the same page width as details on a wide display', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1920, height: 1000 });
  await discoverFixture(page);
  await page.goto('/');
  const search = page.getByRole('search');
  await expect(search).toBeVisible();
  const discoverBounds = await search.boundingBox();
  await page
    .getByRole('button', { name: 'View Arrival', exact: true })
    .first()
    .click();
  const hero = page.locator('article > div').first();
  await expect(hero).toBeVisible();
  const detailsBounds = await hero.boundingBox();
  expect(discoverBounds).not.toBeNull();
  expect(detailsBounds).not.toBeNull();
  expect(discoverBounds!.x).toBeCloseTo(detailsBounds!.x, 0);
  expect(discoverBounds!.width).toBeCloseTo(detailsBounds!.width, 0);
});

test('season requests exclude available seasons and preserve explicit selection on mobile', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const fixture = await discoverFixture(page);
  await page.goto('/#discover/tv/22');
  await expect(
    page.getByRole('heading', { name: 'Severance', exact: true }),
  ).toBeVisible();
  await expect(page.getByRole('checkbox', { name: /Season 1/ })).toBeDisabled();
  await page.getByRole('checkbox', { name: /Season 3/ }).uncheck();
  await page
    .getByRole('button', { name: 'Request 1 season', exact: true })
    .click();
  await expect(page.getByText('Request sent for approval.')).toBeVisible();
  expect(fixture.requests).toEqual([
    { media_type: 'tv', media_id: 22, seasons: [2] },
  ]);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(fixture.errors).toEqual([]);
});

test('late search responses cannot replace a newer query', async ({ page }) => {
  const fixture = await discoverFixture(page);
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route('**/api/v1/seerr/search?**', async (route) => {
    const old =
      new URL(route.request().url()).searchParams.get('q') === 'Arrival';
    if (old) await held;
    await route.fulfill({ json: results(old ? [movie] : [show]) });
  });
  await page.goto('/');
  const input = page.getByRole('textbox', { name: 'Search movies and series' });
  const first = page.waitForRequest('**/seerr/search?q=Arrival*');
  await input.fill('Arrival');
  await first;
  await input.fill('Severance');
  await expect(
    page.getByRole('button', { name: 'View Severance', exact: true }),
  ).toBeVisible();
  release();
  await expect(
    page.getByRole('button', { name: 'View Arrival', exact: true }),
  ).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
});

test('regular users see their requests without administrator approval controls', async ({
  page,
}) => {
  const fixture = await discoverFixture(page);
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Discover navigation' })
    .getByRole('button', { name: 'My requests' })
    .click();
  await expect(page.getByText('Arrival', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Approve', exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Cancel', exact: true }),
  ).toBeVisible();
  expect(fixture.errors).toEqual([]);
});

test('service defaults save eagerly and roll back a rejected edit', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const writes: unknown[] = [];
  let reject = false;
  await page.route(
    '**/api/v1/admin/managers/manager-radarr/defaults',
    async (route) => {
      writes.push(route.request().postDataJSON());
      await route.fulfill(
        reject
          ? { status: 409, json: { error: { message: 'Profile was removed' } } }
          : { json: { saved: true } },
      );
    },
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: /Radarr/ })
    .click();
  const form = page.getByRole('form', { name: 'Radarr acquisition defaults' });
  const monitor = form.getByRole('switch', {
    name: 'Monitor and search requests',
  });
  await expect(monitor).toBeChecked();
  await monitor.press('Space');
  await expect.poll(() => writes.length).toBe(1);
  await expect(monitor).not.toBeChecked();
  reject = true;
  await monitor.press('Space');
  await expect(form.getByRole('alert')).toContainText('Profile was removed');
  await expect(monitor).not.toBeChecked();
  await expect(
    form.getByRole('button', { name: 'Save', exact: true }),
  ).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
});

test('custom profiles retain the chosen quality order and become the selected default', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const created: Record<string, unknown>[] = [];
  await page.route(
    '**/api/v1/admin/managers/manager-radarr/quality-profiles',
    async (route) => {
      if (route.request().method() === 'POST') {
        created.push(route.request().postDataJSON());
        return route.fulfill({ json: { id: 7, name: 'My profile' } });
      }
      return route.fulfill({
        json: {
          qualities: [
            { id: 3, name: 'WEBDL-1080p', resolution: 1080 },
            { id: 4, name: 'Bluray-1080p', resolution: 1080 },
            { id: 5, name: 'WEBDL-2160p', resolution: 2160 },
          ],
        },
      });
    },
  );
  await page.route(
    '**/api/v1/admin/managers/manager-radarr/options',
    async (route) =>
      route.fulfill({
        json: {
          roots: [{ id: 1, path: '/media/movies' }],
          profiles: [
            { id: 1, name: 'HD-1080p' },
            ...(created.length ? [{ id: 7, name: 'My profile' }] : []),
          ],
          metadata_profiles: [],
          defaults: {
            root_folder: '/media/movies',
            quality_profile: created.length ? 7 : 1,
            metadata_profile: null,
            monitored: true,
          },
        },
      }),
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: /Radarr/ })
    .click();
  await page
    .getByRole('button', { name: 'Create quality profile', exact: true })
    .click();
  const form = page.getByRole('form', { name: 'New quality profile' });
  await form.getByPlaceholder('Profile name').fill('My profile');
  await form
    .getByRole('button', { name: 'Reorder Bluray-1080p' })
    .press('ArrowDown');
  await form.getByRole('button', { name: 'Create and use profile' }).click();
  await expect(
    page.getByRole('combobox', { name: 'Quality profile', exact: true }),
  ).toHaveValue('7');
  expect(created).toEqual([
    { name: 'My profile', qualities: [4, 3, 5], cutoff: 5, upgrades: true },
  ]);
  expect(fixture.errors).toEqual([]);
});

test('indexer onboarding loads provider addresses and retains a rejected draft', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const submissions: { path: string; body: Record<string, unknown> }[] = [];
  await page.route(
    '**/api/v1/admin/support/support-prowlarr/indexers/**',
    async (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/schema'))
        return route.fulfill({
          json: {
            profiles: [{ id: 1, name: 'Standard' }],
            items: [
              {
                name: 'Fixture provider',
                implementation: 'Cardigann',
                protocol: 'torrent',
                privacy: 'private',
                fields: [
                  {
                    name: 'definitionFile',
                    hidden: 'hidden',
                    type: 'textbox',
                    value: 'fixture',
                  },
                  {
                    name: 'baseUrl',
                    label: 'Website',
                    type: 'select',
                    selectOptionsProviderAction: 'getUrls',
                  },
                  { name: 'apiKey', label: 'API key', type: 'password' },
                  {
                    name: 'info',
                    label: 'Help',
                    type: 'info',
                    value: '<b>Use your provider credentials</b>',
                  },
                ],
              },
            ],
          },
        });
      if (path.endsWith('/fields'))
        return route.fulfill({
          json: {
            options: [
              {
                value: 'https://indexer.example.test/',
                name: 'Primary address',
              },
            ],
          },
        });
      submissions.push({ path, body: route.request().postDataJSON() });
      return route.fulfill({
        status: 409,
        json: {
          error: {
            message:
              'Invalid API key. Check your indexer API key and try again.',
          },
        },
      });
    },
  );
  await page.route(
    '**/api/v1/admin/support/support-prowlarr/indexers',
    async (route) => {
      submissions.push({
        path: new URL(route.request().url()).pathname,
        body: route.request().postDataJSON(),
      });
      await route.fulfill({ json: { id: 2 } });
    },
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: /Prowlarr/ })
    .click();
  await page.getByRole('button', { name: 'Add indexer', exact: true }).click();
  await page.getByRole('button', { name: /Fixture provider/ }).click();
  await expect(page.getByLabel('Website')).toHaveValue(
    'https://indexer.example.test/',
  );
  await expect(page.getByText('Use your provider credentials')).toBeVisible();
  await page.getByLabel('API key', { exact: true }).fill('fixture-secret');
  await page
    .getByRole('button', { name: 'Test connection', exact: true })
    .click();
  await expect(page.getByRole('alert')).toContainText(
    'Invalid API key. Check your indexer API key and try again.',
  );
  await expect(page.getByLabel('API key', { exact: true })).toHaveValue(
    'fixture-secret',
  );
  await page
    .getByRole('dialog')
    .getByRole('button', { name: 'Add indexer', exact: true })
    .click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(submissions).toHaveLength(2);
  expect(submissions[1].body.fields).toMatchObject({
    baseUrl: 'https://indexer.example.test/',
    apiKey: 'fixture-secret',
    definitionFile: 'fixture',
  });
  expect(fixture.errors).toEqual([]);
});
