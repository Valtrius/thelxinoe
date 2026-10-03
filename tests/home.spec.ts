import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

const video = (id: string, title: string, position = 0, last_played = 0) => ({
  videoId: id,
  channelId: 'subscribed',
  channelName: 'Channel',
  title,
  publishedAt: '2026-10-03T10:00:00Z',
  durationSeconds: 600,
  broadcastState: 'none',
  isLive: false,
  isUpcoming: false,
  isLiveReplay: false,
  positionSeconds: position,
  watchedPercentage: position / 6,
  isWatched: false,
  availabilityStatus: 'available',
  last_played,
});

async function homeFixture(page: Page) {
  const fixture = await installUiFixture(page, {
    section: 'Home',
    preserveNavigation: true,
  });
  const data = {
    library: {
      continue_watching: [
        {
          id: 'movie',
          kind: 'movie',
          title: 'Film',
          available: true,
          position: 120,
          duration: 600,
          fileId: 'edition-file',
          last_played: 200,
        },
      ],
      next_up: [
        {
          id: 'episode',
          kind: 'episode',
          title: 'Next episode',
          show_title: 'Series',
          available: true,
        },
      ],
      watch_later: [
        { id: 'saved', kind: 'movie', title: 'Saved film', available: true },
      ],
      favorites: [
        {
          id: 'favorite',
          kind: 'show',
          title: 'Favorite series',
          available: true,
        },
      ],
    },
    youtube: {
      continue_watching: [video('abcdefghijk', 'Direct-link video', 80, 300)],
      next_up: [video('lmnopqrstuv', 'New subscription upload')],
    },
    providers: { youtube: true, twitch: true, kick: true },
    connected: true,
    twitchFailed: false,
    kick: [
      {
        slug: 'tracked',
        display_name: 'Kick channel',
        title: 'Kick show',
        live: true,
        category: 'Games',
        viewers: 900,
        updated_at: Math.floor(Date.now() / 1000),
        error: null,
        started_at: new Date().toISOString(),
      },
    ],
  };
  await page.route('**/api/v1/me/home', (route) =>
    route.fulfill({ json: data.library }),
  );
  await page.route('**/api/v1/online/providers', (route) =>
    route.fulfill({ json: data.providers }),
  );
  for (const platform of ['youtube', 'twitch'])
    await page.route(`**/api/v1/online/${platform}`, (route) =>
      route.fulfill({
        json: {
          configured: true,
          account: { status: data.connected ? 'connected' : 'disconnected' },
          sync: { last_complete: Math.floor(Date.now() / 1000), error: null },
        },
      }),
    );
  await page.route('**/api/v1/online/youtube/home', (route) =>
    route.fulfill({ json: data.youtube }),
  );
  await page.route('**/api/v1/online/twitch/feed', (route) =>
    data.twitchFailed
      ? route.fulfill({
          status: 503,
          json: { error: { message: 'Twitch is unavailable' } },
        })
      : route.fulfill({
          json: {
            items: [
              {
                id: 'twitch-id',
                login: 'followed',
                display_name: 'Twitch channel',
                title: 'Twitch show',
                category: 'Games',
                viewers: 2100,
                started_at: new Date().toISOString(),
                fetched_at: Math.floor(Date.now() / 1000),
              },
            ],
          },
        }),
  );
  await page.route('**/api/v1/online/kick', (route) =>
    route.fulfill({
      json: {
        connected: data.connected,
        configured: true,
        items: data.kick,
      },
    }),
  );
  return { fixture, data };
}

test('Home combines progress, subscription uploads and live channels on desktop and mobile', async ({
  page,
}, testInfo) => {
  const { fixture } = await homeFixture(page);
  await page.goto('/');
  await expect(page.locator('.page-header h1')).toHaveText('Home');
  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  expect(
    (await nav.getByRole('link').allTextContents())
      .slice(0, 2)
      .map((s) => s.trim()),
  ).toEqual(['Home', 'Discover']);
  const resume = page.getByRole('region', {
    name: 'Continue watching',
    exact: true,
  });
  await expect(resume.locator('h3')).toHaveText(['Direct-link video', 'Film']);
  await expect(
    page.getByRole('region', { name: 'YouTube subscriptions', exact: true }),
  ).toContainText('New subscription upload');
  const live = page.getByRole('region', { name: 'Live now', exact: true });
  await expect(live.locator('h3')).toHaveText([
    'Twitch channel',
    'Kick channel',
  ]);
  await expect(live).toContainText('Twitch');
  await expect(live).toContainText('Kick');
  for (const width of [1440, 1280, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await page.locator('.workspace-scroll').evaluate((node) => {
      node.scrollTop = 0;
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    if (width > 390)
      await expect(
        resume.getByRole('button', { name: 'Next Continue watching' }),
      ).toHaveCount(0);
    else {
      await expect(
        resume.getByRole('button', { name: 'Previous Continue watching' }),
      ).toBeDisabled();
      await expect(
        resume.getByRole('button', { name: 'Next Continue watching' }),
      ).toBeEnabled();
      await expect(
        page
          .getByRole('region', { name: 'Next up', exact: true })
          .getByRole('button', { name: 'Next Next up' }),
      ).toHaveCount(0);
    }
    const path = testInfo.outputPath(`home-${width}.png`);
    await page.screenshot({ path, fullPage: true, animations: 'disabled' });
    await testInfo.attach(`home-${width}`, { path, contentType: 'image/png' });
    for (const label of ['Live now', 'Favorites']) {
      await page
        .getByRole('region', { name: label, exact: true })
        .scrollIntoViewIfNeeded();
      const shelfPath = testInfo.outputPath(
        `home-${width}-${label.toLowerCase().replaceAll(' ', '-')}.png`,
      );
      await page.screenshot({
        path: shelfPath,
        fullPage: true,
        animations: 'disabled',
      });
      await testInfo.attach(`home-${width}-${label}`, {
        path: shelfPath,
        contentType: 'image/png',
      });
    }
  }
  const next = resume.getByRole('button', { name: 'Next Continue watching' });
  const previous = resume.getByRole('button', {
    name: 'Previous Continue watching',
  });
  await next.click();
  await expect(next).toBeDisabled();
  await expect(previous).toBeEnabled();
  await expect
    .poll(() =>
      resume.locator('.grid-flow-col').evaluate((node) => node.scrollLeft),
    )
    .toBeGreaterThan(0);
  await previous.click();
  await expect(previous).toBeDisabled();
  await expect(next).toBeEnabled();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Home hides empty sections and disconnected caches while source failures remain independent', async ({
  page,
}, testInfo) => {
  const { fixture, data } = await homeFixture(page);
  data.twitchFailed = true;
  await page.goto('/');
  await expect(
    page.getByRole('region', { name: 'Continue watching', exact: true }),
  ).toContainText('Film');
  await expect(
    page.getByRole('region', { name: 'Live now', exact: true }),
  ).toContainText('Kick channel');
  await expect(page.getByText(/Twitch is unavailable/)).toBeVisible();
  data.library = {
    continue_watching: [],
    next_up: [],
    watch_later: [],
    favorites: [],
  };
  await page.getByRole('button', { name: 'Refresh Home', exact: true }).click();
  await expect(
    page.getByRole('region', { name: 'YouTube subscriptions', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Next up', exact: true }),
  ).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('home-youtube-only.png'),
    fullPage: true,
    animations: 'disabled',
  });
  data.connected = false;
  await page.getByRole('button', { name: 'Refresh Home', exact: true }).click();
  for (const title of [
    'Continue watching',
    'Next up',
    'YouTube subscriptions',
    'Live now',
    'Watch later',
    'Favorites',
  ])
    await expect(
      page.getByRole('region', { name: title, exact: true }),
    ).toHaveCount(0);
  await expect(
    page.getByRole('navigation', { name: 'Browse media' }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('home-empty.png'),
    fullPage: true,
  });
  data.providers = { youtube: false, twitch: false, kick: false };
  const requested: string[] = [];
  page.on('request', (request) => {
    if (
      /\/online\/(youtube|twitch|kick)(\/|$)/.test(
        new URL(request.url()).pathname,
      )
    )
      requested.push(request.url());
  });
  await page.reload();
  await expect(
    page.getByRole('navigation', { name: 'Browse media' }),
  ).toBeVisible();
  expect(requested).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Home actions preserve resume positions and local editions, and launch live streams', async ({
  page,
}, testInfo) => {
  const { fixture } = await homeFixture(page);
  const launches: { media_id: string; file_id?: string; position?: number }[] =
    [];
  await page.route('**/api/v1/catalog/*/playback', (route) =>
    route.fulfill({
      json: {
        sources: [],
        progress: [],
        watched: false,
        preferences: {
          quality: 'auto',
          audio_language: 'eng',
          subtitle_language: 'eng',
          subtitles: false,
          replay_gain: 'track',
        },
      },
    }),
  );
  await page.route('**/api/v1/playback', (route) => {
    launches.push(route.request().postDataJSON());
    return route.fulfill({
      status: 503,
      json: { error: { message: 'Playback fixture unavailable' } },
    });
  });
  await page.goto('/');
  for (const [region, title, expected] of [
    [
      'Continue watching',
      'Film',
      { media_id: 'movie', file_id: 'edition-file', position: 120 },
    ],
    [
      'Continue watching',
      'Direct-link video',
      { media_id: 'youtube:abcdefghijk', position: 80 },
    ],
    [
      'YouTube subscriptions',
      'New subscription upload',
      { media_id: 'youtube:lmnopqrstuv', position: 0 },
    ],
  ] as const) {
    await page
      .getByRole('region', { name: region, exact: true })
      .getByRole('button', { name: `Play ${title}`, exact: true })
      .click();
    await expect.poll(() => launches.at(-1)).toMatchObject(expected);
    await page
      .getByRole('button', { name: 'Close player', exact: true })
      .click();
  }
  await page
    .getByRole('button', { name: 'Watch Twitch channel', exact: true })
    .click();
  await expect
    .poll(() => launches.at(-1))
    .toMatchObject({ media_id: 'twitch:twitch-id' });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page
    .getByRole('button', { name: 'Watch Kick channel', exact: true })
    .click();
  await expect
    .poll(() => launches.at(-1))
    .toMatchObject({ media_id: 'kick:tracked' });
  await page.screenshot({ path: testInfo.outputPath('home-live-launch.png') });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Home and Discover preserve default, saved and detail-link navigation', async ({
  page,
}) => {
  const { fixture } = await homeFixture(page);
  await page.addInitScript(() => {
    if (!sessionStorage.getItem('home-navigation-started')) {
      localStorage.removeItem('thelxinoe::layout-fixture:navigation');
      sessionStorage.setItem('home-navigation-started', '1');
    }
  });
  await page.route('**/api/v1/seerr/movie/11', (route) =>
    route.fulfill({
      json: { id: 11, title: 'Linked movie', mediaType: 'movie' },
    }),
  );
  await page.route('**/api/v1/seerr/movie/11/recommendations', (route) =>
    route.fulfill({ json: { results: [], totalPages: 1 } }),
  );
  await page.route(/\/api\/v1\/seerr\/profiles\/movie(?:\?|$)/, (route) =>
    route.fulfill({ json: { locked: false, items: [] } }),
  );
  await page.goto('/');
  await expect(page.locator('.page-header h1')).toHaveText('Home');
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Discover', exact: true })
    .click();
  await expect(page.locator('.page-header h1')).toHaveText('Discover');
  await page.reload();
  await expect(page.locator('.page-header h1')).toHaveText('Discover');
  await page.getByRole('link', { name: 'Thelxinoe home' }).click();
  await expect(page.locator('.page-header h1')).toHaveText('Home');
  await page.goto('/?section=Movies#discover/movie/11');
  await expect(page.locator('.page-header h1')).toHaveText('Discover');
  await expect(page.getByText('Linked movie', { exact: true })).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
