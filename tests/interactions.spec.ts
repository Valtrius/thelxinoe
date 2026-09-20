import { test, expect, type Page, type BrowserContext } from '@playwright/test';

// Requires an explicitly selected disposable server. Provider metadata/playback
// are fixtures; sessions, preferences, watchlists and WebSocket events are real.
const headers = { 'X-Thelxinoe-Client': '1' };
const credentials = {
  username: 'admin',
  password: 'test-only long passphrase',
};
const ids = ['testvideo00', 'testvideo01', 'testvideo02'];

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

async function login(context: BrowserContext) {
  const setup = await (await context.request.get('/api/v1/setup')).json();
  const response = await context.request.post(
    setup.setup_required ? '/api/v1/setup' : '/api/v1/auth/login',
    { headers, data: credentials },
  );
  expect(response.ok()).toBe(true);
}
async function providers(page: Page) {
  await page.route('**/api/v1/online/youtube', async (route) => {
    const response = await route.fetch();
    const value = await response.json();
    await route.fulfill({
      json: {
        ...value,
        account: {
          status: 'connected',
          display_name: 'Fixture',
          updated_at: 1,
        },
        sync: {
          in_progress: true,
          phase: 'subscriptions',
          next_run: 9999999999,
          failures: 0,
        },
        quota: { blocked: false },
      },
    });
  });
  await page.route('**/api/v1/online/youtube/watchlists', async (route) => {
    if (route.request().method() !== 'GET') return route.continue();
    const response = await route.fetch();
    const lists = await response.json();
    for (const list of lists)
      for (const item of list.items) {
        item.metadataPending = false;
        Object.assign(item.video, {
          metadataPending: false,
          availabilityStatus: 'available',
          title: `Fixture ${item.video.videoId}`,
          durationSeconds: 60,
        });
      }
    await route.fulfill({ json: lists });
  });
}
async function seed(page: Page) {
  const lists = await (
    await page.request.get('/api/v1/online/youtube/watchlists')
  ).json();
  const id = lists.find((list: { isDefault: boolean }) => list.isDefault).id;
  for (const videoId of ids)
    await page.request.post(`/api/v1/online/youtube/watchlists/${id}/items`, {
      headers,
      data: { videoId, manualPosition: ids.indexOf(videoId) },
    });
  await page.request.put(`/api/v1/online/youtube/watchlists/${id}/order`, {
    headers,
    data: { videoIds: ids },
  });
  await page.request.patch('/api/v1/me/appearance', {
    headers,
    data: {
      provider_preferences: {
        'youtube-watchlist-sidebar-open': 'true',
        'youtube-selected-watchlist-id': String(id),
        'youtube-feed-layout': '{}',
      },
      audio_volume: 0.31,
    },
  });
  return id;
}
async function open(page: Page) {
  await providers(page);
  await page.goto('/?section=YouTube');
  await expect(page.locator('[data-watchlist-video-id]')).toHaveCount(3);
}
const order = (page: Page) =>
  page
    .locator('[data-watchlist-video-id]')
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute('data-watchlist-video-id')),
    );
async function dragFirstToEnd(page: Page) {
  const rows = page.locator('[data-watchlist-video-id]');
  const first = (await rows.first().boundingBox())!;
  const last = (await rows.last().boundingBox())!;
  await page.mouse.move(first.x + 40, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(first.x + 40, first.y + first.height / 2 + 10, {
    steps: 3,
  });
  await page.mouse.move(last.x + 40, last.y + last.height - 2, { steps: 12 });
  await page.mouse.up();
}

test('watchlist order is immediate, rolls back on failure, and synchronizes with another session', async ({
  page,
  browser,
  baseURL,
}) => {
  await login(page.context());
  const id = await seed(page);
  await open(page);
  const other = await browser.newContext({ baseURL });
  try {
    await login(other);
    const second = await other.newPage();
    await open(second);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    let pending = false;
    await page.route(`**/watchlists/${id}/order`, async (route) => {
      pending = true;
      await gate;
      await route.continue();
    });
    await dragFirstToEnd(page);
    await expect.poll(() => pending).toBe(true);
    await expect.poll(() => order(page)).toEqual([ids[1], ids[2], ids[0]]);
    await expect.poll(() => order(second)).toEqual(ids);
    // A refresh from another tab while the write is pending must not snap back.
    const refreshed = page.waitForResponse(
      (response) =>
        response.url().endsWith('/online/youtube/watchlists') &&
        response.request().method() === 'GET',
    );
    await second.request.put(`/api/v1/online/youtube/watchlists/${id}`, {
      headers,
      data: {
        name: 'Watch Later',
        autoDownload: false,
        autoRemoveWatched: false,
        sortMode: 'manual',
        sortDirection: 'asc',
      },
    });
    await refreshed;
    expect(await order(page)).toEqual([ids[1], ids[2], ids[0]]);
    release();
    await expect.poll(() => order(second)).toEqual([ids[1], ids[2], ids[0]]);
    await page.unroute(`**/watchlists/${id}/order`);
    let reject!: () => void;
    const failure = new Promise<void>((resolve) => {
      reject = resolve;
    });
    await page.route(`**/watchlists/${id}/order`, async (route) => {
      await failure;
      await route.fulfill({
        status: 503,
        json: { error: { code: 'fixture', message: 'Order save rejected' } },
      });
    });
    await dragFirstToEnd(page);
    await expect.poll(() => order(page)).toEqual([ids[2], ids[0], ids[1]]);
    reject();
    await expect.poll(() => order(page)).toEqual([ids[1], ids[2], ids[0]]);
    await expect(
      page.getByText('Order save rejected', { exact: true }).first(),
    ).toBeVisible();
  } finally {
    await other.close();
  }
});

test('filters synchronize, refresh stays usable during sync, switches use the shared control, and navigation restores', async ({
  page,
  browser,
  baseURL,
}) => {
  await login(page.context());
  await seed(page);
  await open(page);
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const other = await browser.newContext({ baseURL });
  try {
    await login(other);
    const second = await other.newPage();
    await open(second);
    await page
      .getByRole('button', { name: 'Filter feed', exact: true })
      .click();
    await second
      .getByRole('button', { name: 'Filter feed', exact: true })
      .click();
    await page.getByRole('button', { name: /^Shorts / }).click();
    await expect(
      second.getByRole('button', { name: /^Shorts / }),
    ).toHaveAttribute('aria-pressed', 'true');
    await second.getByRole('button', { name: /^Live / }).click();
    await expect(page.getByRole('button', { name: /^Live / })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    await page.keyboard.press('Escape');
    await expect(
      page.getByRole('button', { name: 'Refresh YouTube feed', exact: true }),
    ).toBeEnabled();
    await page
      .getByRole('button', { name: 'Refresh YouTube feed', exact: true })
      .click();
    await expect(
      page.getByRole('button', { name: 'Refresh YouTube feed', exact: true }),
    ).toBeEnabled();
    await page
      .getByRole('button', { name: 'Watchlist settings', exact: true })
      .click();
    await expect(page.getByRole('switch').first()).toBeVisible();
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Twitch', exact: true }).click();
    await page.reload();
    await expect(page.locator('.page-header h1')).toHaveText('Twitch');
    await page.goto('/');
    await expect(page.locator('.page-header h1')).toHaveText('Twitch');
    expect(
      (await page.locator('.page-header').boundingBox())!.height,
    ).toBeLessThan(55);
    expect(errors).toEqual([]);
  } finally {
    await other.close();
  }
});

test('clicking a watchlist card starts playback and the next player retains its volume', async ({
  page,
}) => {
  await login(page.context());
  await seed(page);
  const starts: string[] = [];
  const preferences = {
    quality: 'auto',
    subtitles: false,
    audio_language: '',
    subtitle_language: '',
    replay_gain: 'off',
  };
  await page.route('**/api/v1/catalog/youtube:*/playback', (route) =>
    route.fulfill({
      json: { sources: [], preferences, progress: [], watched: false },
    }),
  );
  await page.route('**/api/v1/playback', async (route) => {
    starts.push(route.request().postDataJSON().media_id);
    await route.fulfill({
      json: {
        id: 'fixture',
        file_id: 'fixture',
        generation: 'fixture',
        mode: 'direct',
        url: '/fixture.wav',
        position: 0,
        duration: 60,
        timeline_start: 0,
        video: true,
        tracks: [],
        subtitles: [],
        selected_subtitle: 'off',
        options: {},
        probe: {},
        replay_gain: 'off',
      },
    });
  });
  await page.route('**/api/v1/playback/fixture/**', (route) =>
    route.fulfill({ json: {} }),
  );
  await page.route('**/api/v1/catalog/*/segments?*', (route) =>
    route.fulfill({
      json: { generation: 'fixture', items: [], preferences: {} },
    }),
  );
  await open(page);
  await page
    .locator(`[data-watchlist-video-id="${ids[0]}"] [role="button"]`)
    .click({ position: { x: 30, y: 30 } });
  await expect.poll(() => starts).toEqual([`youtube:${ids[0]}`]);
  const volume = page.getByRole('slider', { name: 'Playback volume' });
  await expect(volume).toHaveValue('0.31');
  await volume.fill('0.23');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .audio_volume,
    )
    .toBe(0.23);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page
    .locator(`[data-watchlist-video-id="${ids[1]}"] [role="button"]`)
    .click({ position: { x: 30, y: 30 } });
  await expect(volume).toHaveValue('0.23');
  await expect
    .poll(() =>
      page.locator('video').evaluate((video: HTMLVideoElement) => video.volume),
    )
    .toBe(0.23);
  await page.evaluate(
    (video_id) =>
      window.dispatchEvent(
        new CustomEvent('thelxinoe-download-progress', {
          detail: {
            video_id,
            state: 'downloading',
            downloaded_bytes: 25,
            total_bytes: 100,
            media_kind: 'video',
          },
        }),
      ),
    ids[2],
  );
  await expect(
    page.locator(`[data-watchlist-video-id="${ids[2]}"]`),
  ).toContainText('25%');
});
