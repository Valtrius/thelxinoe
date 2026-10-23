import { expect, test, type Page } from '@playwright/test';
import { installServerEvents, installUiFixture } from './helpers/ui-fixture';
import { responseGate } from './helpers/response-gate';

const navigate = (page: Page, name: string) =>
  page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name, exact: true })
    .click();
const movie = (title: string) => ({
  id: 'cached-movie',
  kind: 'movie',
  title,
  year: 2026,
  available: true,
  watched: false,
  metadata: {},
});
test.afterEach(async ({ page }) =>
  page.unrouteAll({ behavior: 'ignoreErrors' }),
);

test('library returns reuse recent data and show stale cards while the next response is pending', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  const gate = responseGate();
  let reads = 0,
    hold = false;
  await page.route('**/api/v1/catalog/collections', (route) =>
    route.fulfill({
      json: { items: [{ id: 1, name: 'Saved collection', count: 1 }] },
    }),
  );
  await page.route('**/api/v1/catalog?kind=movie&**', async (route) => {
    reads++;
    if (hold) await gate.promise;
    await route.fulfill({
      json: { items: [movie(hold ? 'Updated movie' : 'Cached movie')] },
    });
  });
  try {
    await page.goto('/');
    await expect(page.getByText('Cached movie', { exact: true })).toBeVisible();
    await navigate(page, 'Shows');
    await navigate(page, 'Movies');
    await expect(page.getByText('Cached movie', { exact: true })).toBeVisible();
    expect(reads).toBe(1);
    await navigate(page, 'Shows');
    await page.clock.install();
    await page.clock.runFor(31_000);
    hold = true;
    await navigate(page, 'Movies');
    await expect.poll(() => reads).toBe(2);
    await expect(page.getByText('Cached movie', { exact: true })).toBeVisible();
    await expect(
      page.getByRole('combobox', { name: 'Collection', exact: true }),
    ).toBeVisible();
    gate.release();
    await expect(
      page.getByText('Updated movie', { exact: true }),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

test('catalog changes received while away invalidate the saved list', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  const events = await installServerEvents(page);
  let reads = 0;
  await page.route('**/api/v1/catalog?kind=movie&**', (route) => {
    reads++;
    return route.fulfill({
      json: { items: [movie(reads === 1 ? 'Before change' : 'After change')] },
    });
  });
  await page.goto('/');
  await expect(page.getByText('Before change', { exact: true })).toBeVisible();
  await expect.poll(events.connected).toBe(true);
  await navigate(page, 'Shows');
  events.send('catalog.changed');
  await navigate(page, 'Movies');
  await expect(page.getByText('After change', { exact: true })).toBeVisible();
  expect(reads).toBe(2);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Discover retains loaded rows across navigation without fetching hidden rows eagerly', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
  const reads: string[] = [];
  await page.route('**/api/v1/seerr/status', (route) =>
    route.fulfill({ json: { configured: true, ready: true } }),
  );
  await page.route('**/api/v1/seerr/discover/*', (route) => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1)!;
    reads.push(id);
    return route.fulfill({
      json: {
        results: [
          {
            id: 1,
            mediaType: 'movie',
            title: `Cached ${id}`,
            posterPath: null,
          },
        ],
        page: 1,
        totalPages: 1,
        totalResults: 1,
      },
    });
  });
  await page.goto('/');
  await expect(
    page.getByText('Cached trending', { exact: true }),
  ).toBeVisible();
  await navigate(page, 'Movies');
  await navigate(page, 'Discover');
  await expect(
    page.getByText('Cached trending', { exact: true }),
  ).toBeVisible();
  expect(reads.filter((id) => id === 'trending')).toHaveLength(1);
  expect(reads).not.toContain('upcoming-tv');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('live channels reappear while their account is revalidated', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Twitch' });
  const gate = responseGate();
  let hold = false,
    reads = 0;
  await page.route('**/api/v1/online/twitch', async (route) => {
    if (hold) await gate.promise;
    await route.fulfill({
      json: {
        configured: true,
        account: {
          status: 'connected',
          external_id: 'viewer',
          display_name: 'Viewer',
        },
        sync: { last_complete: 1 },
      },
    });
  });
  await page.route('**/api/v1/online/twitch/feed', (route) => {
    reads++;
    return route.fulfill({
      json: {
        items: [
          {
            id: 'channel',
            login: 'channel',
            display_name: 'Cached channel',
            title: 'Live show',
            category: 'Music',
            viewers: 12,
            started_at: '2026-10-01T10:00:00Z',
          },
        ],
      },
    });
  });
  try {
    await page.goto('/');
    await expect(
      page.getByText('Cached channel', { exact: true }),
    ).toBeVisible();
    await navigate(page, 'Movies');
    hold = true;
    await navigate(page, 'Twitch');
    await expect(
      page.getByText('Cached channel', { exact: true }),
    ).toBeVisible();
    expect(reads).toBe(1);
    gate.release();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

test('signing out clears the prior account navigation data', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  let reads = 0;
  await page.route('**/api/v1/catalog?kind=movie&**', (route) => {
    reads++;
    return route.fulfill({
      json: {
        items: [
          movie(reads === 1 ? 'First account movie' : 'Second account movie'),
        ],
      },
    });
  });
  await page.route('**/api/v1/auth/logout', (route) =>
    route.fulfill({ json: {} }),
  );
  await page.goto('/');
  await expect(
    page.getByText('First account movie', { exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page.route('**/api/v1/auth/login', (route) =>
    route.fulfill({
      json: {
        user: {
          id: 'second-account',
          username: 'Second account',
          role: 'user',
          timezone: 'UTC',
        },
      },
    }),
  );
  await page.getByLabel('Username', { exact: true }).fill('second-account');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test password');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await navigate(page, 'Movies');
  await expect(
    page.getByText('Second account movie', { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText('First account movie', { exact: true }),
  ).toHaveCount(0);
  expect(reads).toBe(2);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('YouTube restores its first page while account revalidation is pending', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  const gate = responseGate();
  let hold = false,
    reads = 0;
  await page.route('**/api/v1/online/youtube', async (route) => {
    if (hold) await gate.promise;
    await route.fulfill({
      json: {
        configured: true,
        account: {
          status: 'connected',
          external_id: 'viewer',
          display_name: 'Viewer',
          updated_at: 0,
        },
        sync: {},
      },
    });
  });
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route('**/api/v1/online/youtube/browse', (route) => {
    reads++;
    return route.fulfill({
      json: {
        items: [
          {
            videoId: 'abcdefghijk',
            channelId: 'channel',
            channelName: 'Channel',
            title: 'Cached upload',
            publishedAt: '2026-10-01T10:00:00Z',
            durationSeconds: 600,
            broadcastState: 'none',
            isLive: false,
            isUpcoming: false,
            isLiveReplay: false,
            positionSeconds: 0,
            watchedPercentage: 0,
            isWatched: false,
            availabilityStatus: 'available',
            thumbnailUrl: null,
          },
        ],
        page: 0,
        pageSize: 60,
        hasMore: false,
        channels: [],
        counts: {
          all: 1,
          unwatched: 1,
          inProgress: 0,
          watched: 0,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 1,
        },
      },
    });
  });
  try {
    await page.goto('/');
    await expect(
      page.getByText('Cached upload', { exact: true }),
    ).toBeVisible();
    await navigate(page, 'Movies');
    hold = true;
    await navigate(page, 'YouTube');
    await expect(
      page.getByText('Cached upload', { exact: true }),
    ).toBeVisible();
    expect(reads).toBe(1);
    gate.release();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});
