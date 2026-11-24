import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

const video = (videoId: string, title: string, availabilityStatus: string) => ({
  videoId,
  channelId: 'channel',
  channelName: 'Channel',
  title,
  publishedAt: '2026-10-01T10:00:00Z',
  durationSeconds: 600,
  broadcastState: 'none',
  isLive: false,
  isUpcoming: false,
  isLiveReplay: false,
  positionSeconds: 0,
  watchedPercentage: 0,
  isWatched: false,
  availabilityStatus,
  thumbnailUrl: `/api/v1/online/youtube/videos/${videoId}/artwork?grant=stable`,
});

async function youtubePage(page: Page, items: ReturnType<typeof video>[]) {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  await page.route('**/api/v1/online/youtube', (route) =>
    route.fulfill({
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
    }),
  );
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route('**/api/v1/online/youtube/browse', (route) =>
    route.fulfill({
      json: {
        items,
        page: 0,
        pageSize: 60,
        hasMore: false,
        channels: [],
        counts: {
          all: items.length,
          unwatched: items.length,
          inProgress: 0,
          watched: 0,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 1,
        },
      },
    }),
  );
  const requests: URL[] = [];
  let failures = 0;
  await page.route('**/api/v1/online/youtube/videos/*/artwork?*', (route) => {
    requests.push(new URL(route.request().url()));
    if (requests.length <= failures)
      return route.fulfill({
        status: 404,
        json: { error: { code: 'not_found', message: 'Not found' } },
      });
    return route.fulfill({
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="100%" height="100%" fill="#555"/></svg>',
    });
  });
  return {
    fixture,
    requests,
    failFirst: (count: number) => (failures = count),
  };
}

const loaded = (page: Page, id: string) =>
  page
    .locator(`article[data-video-id="${id}"] img`)
    .evaluate((img) => (img as HTMLImageElement).naturalWidth);

test('A thumbnail that fails to load is requested again under a new address', async ({
  page,
}) => {
  await page.clock.install();
  const { fixture, requests, failFirst } = await youtubePage(page, [
    video('abcdefghijk', 'Fresh upload', 'available'),
  ]);
  failFirst(1);
  await page.goto('/');
  const card = page.locator('article[data-video-id="abcdefghijk"]');
  await expect(card).toBeVisible();
  await expect.poll(() => requests.length).toBe(1);
  await expect(card.locator('img')).toHaveCount(0);
  await page.clock.runFor(2_000);
  await expect.poll(() => loaded(page, 'abcdefghijk')).toBeGreaterThan(0);
  expect(requests.map((url) => url.searchParams.get('retry'))).toEqual([
    null,
    '1',
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('A thumbnail that keeps failing settles on the placeholder', async ({
  page,
}) => {
  await page.clock.install();
  const { fixture, requests, failFirst } = await youtubePage(page, [
    video('abcdefghijk', 'Missing artwork', 'available'),
  ]);
  failFirst(Infinity);
  await page.goto('/');
  const card = page.locator('article[data-video-id="abcdefghijk"]');
  await expect(card).toBeVisible();
  for (const [delay, count] of [
    [2_000, 2],
    [10_000, 3],
    [30_000, 4],
  ]) {
    await expect.poll(() => requests.length).toBe(count - 1);
    await page.clock.runFor(delay);
  }
  await expect.poll(() => requests.length).toBe(4);
  await page.clock.runFor(60_000);
  expect(requests).toHaveLength(4);
  await expect(card.locator('img')).toHaveCount(0);
  await expect(card.getByText('ABCD', { exact: true })).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('A video YouTube no longer serves is labelled without requesting its thumbnail', async ({
  page,
}) => {
  const { fixture, requests } = await youtubePage(page, [
    video('abcdefghijk', 'Available upload', 'available'),
    video('lmnopqrstuv', 'Removed upload', 'unavailable'),
    {
      ...video('wxyzabcdefg', 'Removed replay', 'unavailable'),
      broadcastState: 'replay',
      isLiveReplay: true,
    },
  ]);
  await page.goto('/');
  await expect.poll(() => loaded(page, 'abcdefghijk')).toBeGreaterThan(0);
  for (const id of ['lmnopqrstuv', 'wxyzabcdefg']) {
    const removed = page.locator(`article[data-video-id="${id}"]`);
    await expect(
      removed.getByText('Unavailable', { exact: true }),
    ).toBeVisible();
    await expect(removed.locator('img')).toHaveCount(0);
  }
  await expect(
    page
      .locator('article[data-video-id="wxyzabcdefg"]')
      .getByText('Live replay', { exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .locator('article[data-video-id="abcdefghijk"]')
      .getByText('Unavailable', { exact: true }),
  ).toHaveCount(0);
  expect(requests.map((url) => url.pathname)).toEqual([
    '/api/v1/online/youtube/videos/abcdefghijk/artwork',
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
