import { test, expect, type Page, type Route } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { installUiFixture } from './helpers/ui-fixture';
import { sources } from '../frontend/src/lib/statistics/helpers';
import type {
  StatisticsOverview,
  StatisticsPlatform,
  StatisticsRange,
} from '../frontend/src/lib/statistics/types';

function overview(
  platform: StatisticsPlatform = 'all',
  multiplier = 1,
): StatisticsOverview {
  const selected = sources.filter(
    (source) => platform === 'all' || source.value === platform,
  );
  const seconds = Object.fromEntries(
    sources.map((source, index) => [
      source.field,
      selected.includes(source) ? (index + 1) * 600 * multiplier : 0,
    ]),
  ) as StatisticsOverview['activity'][number];
  const totals = sources.map((source) => ({
    platform: source.value,
    activeSeconds: seconds[source.field],
  }));
  const total = totals.reduce((sum, source) => sum + source.activeSeconds, 0);
  return {
    range: '30d',
    platform,
    timezone: 'UTC',
    interval: 'day',
    trackingStartedAt: '2026-09-01T00:00:00Z',
    totalActiveSeconds: total,
    estimatedActiveSeconds: 0,
    activeDays: multiplier ? 3 : 0,
    periodDays: 30,
    averageActiveSecondsPerDay: total / 3,
    youtubeVideosStarted: 3,
    youtubeVideosWatched: 2,
    twitchChannelsWatched: 1,
    kickChannelsWatched: 1,
    moviesStarted: 2,
    moviesWatched: 1,
    episodesStarted: 3,
    episodesWatched: 2,
    showsWatched: 1,
    tracksStarted: 4,
    tracksCompleted: 3,
    artistsListened: 2,
    albumsListened: 3,
    activity: Array.from({ length: 30 }, (_, index) => ({
      ...seconds,
      periodStart: `2026-09-${String(index + 1).padStart(2, '0')}`,
    })),
    rhythm: Array.from({ length: 168 }, (_, index) => ({
      ...seconds,
      weekday: Math.floor(index / 24),
      hour: index % 24,
    })),
    platformTotals: totals,
    topChannels: selected.map((source) => ({
      platform: source.value,
      id: source.value,
      name: `${source.label} fixture`,
      activeSeconds: seconds[source.field],
      watchedDays: 3,
      contentCount: 3,
    })),
    youtubeContentMix: { uploads: 1, liveReplays: 1, shorts: 0 },
    users: [
      {
        id: 'layout-fixture',
        username: 'Layout viewer',
        activeSeconds: total / 2,
      },
      { id: 'bob', username: 'Bob', activeSeconds: total / 2 },
    ],
  };
}

function historyPayload(title = 'History fixture') {
  return {
    items: [
      {
        id: 'history-fixture',
        media_id: 'movie',
        kind: 'movie',
        title,
        edition: '',
        user_id: 'layout-fixture',
        username: 'Layout viewer',
        device: 'Browser',
        started_at: 1_795_000_000,
        updated_at: 1_795_000_060,
        ended_at: 1_795_000_060,
        position: 60,
        duration: 120,
        played_seconds: 60,
        state: 'stopped',
      },
    ],
    next_before: null,
  };
}

async function fixture(page: Page, role: 'user' | 'admin' = 'user') {
  const base = await installUiFixture(page, { role, section: 'Statistics' });
  const queries: URL[] = [];
  const historyQueries: URL[] = [];
  let override:
    ((route: Route, url: URL) => Promise<void> | undefined) | undefined;
  let historyOverride:
    ((route: Route, url: URL) => Promise<void> | undefined) | undefined;
  await page.route('**/api/v1/users', (route) =>
    route.fulfill({
      json: {
        items: [
          {
            id: 'layout-fixture',
            username: 'Layout viewer',
            role,
            timezone: 'UTC',
          },
          { id: 'bob', username: 'Bob', role: 'user', timezone: 'UTC' },
        ],
      },
    }),
  );
  await page.route('**/api/v1/*/statistics?*', async (route) => {
    const url = new URL(route.request().url());
    queries.push(url);
    const response = override?.(route, url);
    if (response) return response;
    const value = overview(
      url.searchParams.get('platform') as StatisticsPlatform,
      url.pathname.includes('/admin/') && !url.searchParams.has('user') ? 2 : 1,
    );
    value.range = url.searchParams.get('range') as StatisticsRange;
    await route.fulfill({ json: value });
  });
  await page.route('**/api/v1/*/history?*', async (route) => {
    const url = new URL(route.request().url());
    historyQueries.push(url);
    const response = historyOverride?.(route, url);
    if (response) return response;
    await route.fulfill({ json: historyPayload() });
  });
  return {
    ...base,
    queries,
    historyQueries,
    setOverride(next: typeof override) {
      override = next;
    },
    setHistoryOverride(next: typeof historyOverride) {
      historyOverride = next;
    },
  };
}

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

test('legacy History navigation opens Statistics', async ({ page }) => {
  await fixture(page);
  await page.goto('/?section=History');
  await expect(
    page.getByRole('region', { name: 'Statistics summary' }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Playback history', exact: true }),
  ).toBeVisible();
});

test('statistics shows the youtwitch panels with all media, source filters and keyboard-accessible rhythm', async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto('/');
  await expect(
    page.getByRole('region', { name: 'Statistics summary' }),
  ).toBeVisible();
  expect(state.queries[0].pathname).toBe('/api/v1/me/statistics');
  expect(state.queries[0].searchParams.get('platform')).toBe('all');
  await expect(
    page.getByRole('heading', { name: 'Playback history', exact: true }),
  ).toBeVisible();
  expect(state.historyQueries[0].pathname).toBe('/api/v1/me/history');
  expect(state.historyQueries[0].searchParams.get('platform')).toBe('all');
  expect(state.historyQueries[0].searchParams.get('range')).toBe('30d');
  await expect(page.getByLabel('Media')).toHaveCount(0);
  await expect(page.getByLabel('From date (UTC)')).toHaveCount(0);
  await expect(
    page.getByRole('combobox', { name: 'Statistics user' }),
  ).toHaveCount(0);
  for (const title of [
    '01 / DAILY ACTIVITY',
    '02 / PLATFORM SPLIT',
    '03 / VIEWING RHYTHM',
    '04 / TOP CHANNELS & TITLES',
    '05 / LIBRARY COMPLETION',
  ])
    await expect(
      page.getByRole('heading', { name: title, exact: true }),
    ).toBeVisible();
  const grid = page.getByRole('grid', {
    name: 'Watch time by weekday and hour',
  });
  await expect(grid.getByRole('gridcell')).toHaveCount(168);
  const first = grid.getByRole('gridcell').first();
  await first.focus();
  await page.keyboard.press('ArrowRight');
  await expect(grid.getByRole('gridcell').nth(1)).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(grid.getByRole('gridcell').nth(25)).toBeFocused();
  await expect(page.getByRole('tooltip')).toContainText('Tue · 01:00');
  const platforms = page.getByRole('group', { name: 'Statistics platform' });
  await platforms.getByRole('button', { name: 'Music', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: '05 / MUSIC LIBRARY' }),
  ).toBeVisible();
  await expect(
    page.getByText('2 artists · 3 albums', { exact: false }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: '04 / TOP ARTISTS' }),
  ).toBeVisible();
  expect(state.queries.at(-1)?.searchParams.get('platform')).toBe('music');
  await expect
    .poll(() => state.historyQueries.at(-1)?.searchParams.get('platform'))
    .toBe('music');
  await page
    .getByRole('group', { name: 'Statistics range' })
    .getByRole('button', { name: '7D', exact: true })
    .click();
  await expect
    .poll(() => state.queries.at(-1)?.searchParams.get('range'))
    .toBe('7d');
  await expect
    .poll(() => state.historyQueries.at(-1)?.searchParams.get('range'))
    .toBe('7d');
  await page.getByRole('button', { name: 'How this is measured' }).click();
  await expect(page.getByRole('dialog')).toContainText('FILMS / SHOWS / MUSIC');
  await page.keyboard.press('Escape');
  await platforms.getByRole('button', { name: 'All', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: '05 / LIBRARY COMPLETION' }),
  ).toBeVisible();
  mkdirSync('.local/statistics-ui', { recursive: true });
  await page
    .locator('.workspace-scroll')
    .evaluate((element) => (element.scrollTop = 0));
  await page.screenshot({ path: '.local/statistics-ui/desktop.png' });
  await page.setViewportSize({ width: 640, height: 900 });
  await expect
    .poll(() =>
      page.locator('.statistics').evaluate((element) => {
        const workspace = element.closest('.workspace-scroll')!;
        const bounds = workspace.getBoundingClientRect();
        return [...element.children]
          .filter((child) => child.tagName !== 'DIALOG')
          .map((child) => ({
            kind: child.tagName,
            width: Math.round(child.getBoundingClientRect().width),
            right: Math.round(
              child.getBoundingClientRect().right - bounds.right,
            ),
          }))
          .filter((child) => child.right > 1);
      }),
    )
    .toEqual([]);
  await expect(
    page.getByRole('heading', { name: '01 / DAILY ACTIVITY' }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    ),
  ).toBeLessThanOrEqual(1);
  await page.screenshot({ path: '.local/statistics-ui/narrow.png' });
  expect(state.errors).toEqual([]);
  expect(state.unexpected).toEqual([]);
});

test('administrators can aggregate or select a user without stale responses replacing their selection', async ({
  page,
}) => {
  const state = await fixture(page, 'admin');
  await page.goto('/');
  const scope = page.getByRole('combobox', { name: 'Statistics user' });
  await expect(scope).toHaveValue('mine');
  await expect(scope.locator('option[value="bob"]')).toHaveCount(1);
  await scope.selectOption('all');
  await expect(
    page.getByRole('region', { name: 'Watch time by user' }),
  ).toBeVisible();
  expect(state.queries.at(-1)?.pathname).toBe('/api/v1/admin/statistics');
  expect(state.queries.at(-1)?.searchParams.has('user')).toBe(false);
  await expect
    .poll(() => state.historyQueries.at(-1)?.pathname)
    .toBe('/api/v1/admin/history');
  expect(state.historyQueries.at(-1)?.searchParams.has('user')).toBe(false);
  await page
    .getByRole('region', { name: 'Watch time by user' })
    .getByRole('button', { name: /Bob/ })
    .click();
  await expect
    .poll(() => state.queries.at(-1)?.searchParams.get('user'))
    .toBe('bob');
  await expect
    .poll(() => state.historyQueries.at(-1)?.searchParams.get('user'))
    .toBe('bob');
  await expect(
    page.getByRole('region', { name: 'Watch time by user' }),
  ).toHaveCount(0);
  let held: Route | undefined;
  state.setOverride((route, url) => {
    if (url.searchParams.get('platform') === 'movies') {
      held = route;
      return Promise.resolve();
    }
  });
  const platforms = page.getByRole('group', { name: 'Statistics platform' });
  await platforms.getByRole('button', { name: 'Films', exact: true }).click();
  await expect.poll(() => Boolean(held)).toBe(true);
  await platforms.getByRole('button', { name: 'Music', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: '05 / MUSIC LIBRARY' }),
  ).toBeVisible();
  await held!.fulfill({ json: overview('movies') });
  await expect(
    page.getByRole('heading', { name: '05 / MUSIC LIBRARY' }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: '05 / FILM LIBRARY' }),
  ).toHaveCount(0);
  await scope.selectOption('mine');
  await expect
    .poll(() => state.queries.at(-1)?.pathname)
    .toBe('/api/v1/me/statistics');
  await expect
    .poll(() => state.historyQueries.at(-1)?.pathname)
    .toBe('/api/v1/me/history');
  await expect(
    page.getByRole('combobox', { name: 'History scope' }),
  ).toHaveCount(0);
  expect(state.errors).toEqual([]);
  expect(state.unexpected).toEqual([]);
});

test('history ignores stale responses when the administrator scope changes', async ({
  page,
}) => {
  const state = await fixture(page, 'admin');
  await page.goto('/');
  const scope = page.getByRole('combobox', { name: 'Statistics user' });
  let held: Route | undefined;
  state.setHistoryOverride((route, url) => {
    if (
      url.pathname === '/api/v1/admin/history' &&
      !url.searchParams.has('user')
    ) {
      held = route;
      return Promise.resolve();
    }
  });
  await scope.selectOption('all');
  await expect.poll(() => Boolean(held)).toBe(true);
  await scope.selectOption('bob');
  await expect
    .poll(() => state.historyQueries.at(-1)?.searchParams.get('user'))
    .toBe('bob');
  await expect(
    page.getByText('History fixture', { exact: true }),
  ).toBeVisible();
  await held!.fulfill({ json: historyPayload('Stale history') });
  await expect(page.getByText('Stale history', { exact: true })).toHaveCount(0);
  await expect(
    page.getByText('History fixture', { exact: true }),
  ).toBeVisible();
});

test('statistics failures can be retried and older time attribution is explained', async ({
  page,
}) => {
  const state = await fixture(page);
  state.setOverride((route) =>
    route.fulfill({
      status: 503,
      json: { error: { message: 'Statistics temporarily unavailable' } },
    }),
  );
  await page.goto('/');
  await expect(page.getByRole('alert')).toContainText(
    'Statistics temporarily unavailable',
  );
  state.setOverride((route) =>
    route.fulfill({ json: { ...overview(), estimatedActiveSeconds: 120 } }),
  );
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(
    page.getByText('2m from earlier sessions is included.', { exact: false }),
  ).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  state.setOverride((route) => route.fulfill({ json: overview('all', 0) }));
  await page.getByRole('button', { name: 'Refresh statistics' }).click();
  await expect(
    page.getByText('No active playback in this view yet.'),
  ).toBeVisible();
  expect(state.errors).toEqual([]);
  expect(state.unexpected).toEqual([]);
});
