import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { responseGate } from './helpers/response-gate';

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

for (const section of ['Movies', 'Shows', 'Music', 'Playlists'] as const) {
  test(`${section} reserves content while its first response is pending`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section });
    const gate = responseGate();
    await page.route(
      section === 'Playlists' ? '**/api/v1/playlists' : '**/api/v1/catalog?**',
      async (route) => {
        await gate.promise;
        if (section === 'Playlists')
          await route.fulfill({ json: { items: [] } });
        else await route.fallback();
      },
    );
    try {
      await page.goto('/');
      const loading = page.getByRole('status', {
        name: `Loading ${section.toLowerCase()}`,
        exact: true,
      });
      await expect(loading).toBeVisible();
      await expect(page.getByText('No media yet', { exact: true })).toHaveCount(
        0,
      );
      await expect(
        page.getByText('Create a playlist or save tracks from Music.', {
          exact: true,
        }),
      ).toHaveCount(0);
      await testInfo.attach('pending-content', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      gate.release();
      await expect(loading).toHaveCount(0);
      if (section === 'Playlists')
        await expect(
          page.getByText('Create a playlist or save tracks from Music.'),
        ).toBeVisible();
      else
        await expect(
          page.getByRole('heading', { name: 'Fixture movie 0', exact: true }),
        ).toBeVisible();
      expect(fixture.errors).toEqual([]);
      expect(fixture.unexpected).toEqual([]);
    } finally {
      gate.release();
    }
  });
}

for (const platform of ['YouTube', 'Twitch', 'Kick'] as const) {
  test(`${platform} waits for account state before showing connection controls`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: platform });
    await page.route('**/api/v1/online/youtube/watchlists', (route) =>
      route.fulfill({ json: { items: [] } }),
    );
    const gate = responseGate();
    await page.route(
      `**/api/v1/online/${platform.toLowerCase()}`,
      async (route) => {
        await gate.promise;
        await route.fallback();
      },
    );
    try {
      await page.goto('/');
      const loading = page.getByRole('status', {
        name: `Loading ${platform}`,
        exact: true,
      });
      await expect(loading).toBeVisible();
      await expect(
        page.getByRole('button', { name: `Connect ${platform}`, exact: true }),
      ).toHaveCount(0);
      await testInfo.attach('pending-account', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      gate.release();
      await expect(loading).toHaveCount(0);
      await expect(page.locator('[data-feed-content]')).toBeVisible();
      expect(fixture.errors).toEqual([]);
      expect(fixture.unexpected).toEqual([]);
    } finally {
      gate.release();
    }
  });
}

test('Home displays ready sources while the library is pending and keeps content during refresh', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Home' });
  await page.route('**/api/v1/online/providers', (route) =>
    route.fulfill({ json: { youtube: true, twitch: false, kick: false } }),
  );
  await page.route('**/api/v1/online/youtube', (route) =>
    route.fulfill({
      json: { configured: true, account: { status: 'connected' } },
    }),
  );
  await page.route('**/api/v1/online/youtube/home', (route) =>
    route.fulfill({
      json: {
        continue_watching: [],
        next_up: [
          {
            videoId: 'abcdefghijk',
            channelName: 'Channel',
            title: 'Ready upload',
            durationSeconds: 600,
          },
        ],
      },
    }),
  );
  let gate = responseGate();
  await page.route('**/api/v1/me/home', async (route) => {
    await gate.promise;
    await route.fulfill({
      json: {
        continue_watching: [],
        next_up: [
          {
            id: 'episode',
            kind: 'episode',
            title: 'Ready episode',
            available: true,
          },
        ],
        watch_later: [],
        favorites: [],
      },
    });
  });
  try {
    await page.goto('/');
    await expect(
      page.getByRole('status', { name: 'Loading library', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('heading', { name: 'Ready upload', exact: true }),
    ).toBeVisible();
    await testInfo.attach('independent-sources', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    gate.release();
    await expect(
      page.getByRole('heading', { name: 'Ready episode', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('status', { name: 'Loading library', exact: true }),
    ).toHaveCount(0);
    gate = responseGate();
    await page.getByRole('button', { name: 'Refresh Home' }).click();
    await expect(
      page.getByRole('heading', { name: 'Ready episode', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('status', { name: 'Loading library', exact: true }),
    ).toHaveCount(0);
    gate.release();
    await expect(
      page.getByRole('button', { name: 'Refresh Home' }),
    ).toBeEnabled();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

test('Discover displays completed feeds while another feed is pending', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
  const gate = responseGate();
  await page.route('**/api/v1/seerr/status', (route) =>
    route.fulfill({ json: { configured: true, ready: true } }),
  );
  await page.route('**/api/v1/seerr/discover/*', async (route) => {
    const feed = new URL(route.request().url()).pathname.split('/').at(-1);
    if (feed === 'tv') await gate.promise;
    await route.fulfill({
      json: {
        results:
          feed === 'movies'
            ? [{ id: 1, mediaType: 'movie', title: 'Ready discovery' }]
            : [],
      },
    });
  });
  try {
    await page.goto('/');
    await expect(
      page.getByRole('button', { name: /Ready discovery/ }),
    ).toBeVisible();
    await expect(
      page.getByRole('status', { name: 'Loading Popular series', exact: true }),
    ).toBeVisible();
    await testInfo.attach('independent-feeds', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    gate.release();
    await expect(
      page.getByRole('status', { name: 'Loading Popular series', exact: true }),
    ).toHaveCount(0);
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`statistics skeleton fits ${viewport.width}px and yields to an error`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize(viewport);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const fixture = await installUiFixture(page, { section: 'Statistics' });
    const gate = responseGate();
    await page.route('**/api/v1/me/history?**', (route) =>
      route.fulfill({ json: { items: [], next_before: null } }),
    );
    await page.route('**/api/v1/me/statistics?**', async (route) => {
      await gate.promise;
      await route.fulfill({
        status: 503,
        json: { error: { message: 'Viewing history unavailable' } },
      });
    });
    try {
      await page.goto('/');
      await expect(
        page.getByRole('button', { name: 'Refresh statistics', exact: true }),
      ).toBeVisible();
      const loading = page.getByRole('status', {
        name: 'Loading statistics',
        exact: true,
      });
      await expect(loading).toBeVisible();
      expect(
        await loading.evaluate(
          (element) => element.scrollWidth <= element.clientWidth,
        ),
      ).toBe(true);
      expect(
        await loading
          .locator('[data-skeleton]')
          .first()
          .evaluate((element) => getComputedStyle(element).animationName),
      ).toBe('none');
      await testInfo.attach('statistics-loading', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      gate.release();
      await expect(loading).toHaveCount(0);
      await expect(page.getByRole('alert')).toContainText(
        'Viewing history unavailable',
      );
      expect(fixture.errors).toEqual([]);
      expect(fixture.unexpected).toEqual([]);
    } finally {
      gate.release();
    }
  });
}

for (const setting of [
  {
    section: 'devices',
    path: '/auth/sessions',
    label: 'Loading devices',
    heading: 'Your devices',
  },
  {
    section: 'people',
    path: '/users',
    label: 'Loading users',
    heading: 'User access',
  },
  {
    section: 'account',
    path: '/me/preferences',
    label: 'Loading display preferences',
    heading: 'Your display preferences',
  },
] as const) {
  test(`${setting.section} settings keep the shell visible while data loads`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: setting.section,
    });
    const gate = responseGate();
    let reads = 0;
    await page.route(`**/api/v1${setting.path}`, async (route) => {
      if (setting.section !== 'account' || ++reads > 1) await gate.promise;
      await route.fallback();
    });
    try {
      await page.goto('/');
      const loading = page.getByRole('status', {
        name: setting.label,
        exact: true,
      });
      await expect(loading).toBeVisible();
      await expect(
        page.getByRole('heading', { name: setting.heading, exact: true }),
      ).toBeVisible();
      await testInfo.attach('settings-loading', {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
      gate.release();
      await expect(loading).toHaveCount(0);
      expect(fixture.errors).toEqual([]);
      expect(fixture.unexpected).toEqual([]);
    } finally {
      gate.release();
    }
  });
}
