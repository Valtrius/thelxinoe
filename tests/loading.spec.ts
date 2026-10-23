import { expect, test, type WebSocketRoute } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { current } from './helpers/versions';
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

test('switching providers starts the next account read while the previous one is pending', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  const gate = responseGate();
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({ json: { items: [] } }),
  );
  await page.route('**/api/v1/online/youtube', async (route) => {
    await gate.promise;
    await route.fallback();
  });
  try {
    await page.goto('/');
    await expect(
      page.getByRole('status', { name: 'Loading YouTube', exact: true }),
    ).toBeVisible();
    const next = page.waitForResponse('**/api/v1/online/twitch');
    await page
      .getByRole('navigation', { name: 'Main navigation' })
      .getByRole('link', { name: 'Twitch', exact: true })
      .click();
    await next;
    await expect(page.locator('[data-feed-content]')).toBeVisible();
    await expect(page.locator('.page-header h1')).toHaveText('Twitch');
    gate.release();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

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

test('Home appears with the library while YouTube reads its account and feed together', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Home' });
  const account = responseGate();
  let feedRequested = false;
  await page.route('**/api/v1/online/providers', (route) =>
    route.fulfill({ json: { youtube: true, twitch: false, kick: false } }),
  );
  await page.route('**/api/v1/online/youtube', async (route) => {
    await account.promise;
    await route.fulfill({
      json: { configured: true, account: { status: 'connected' } },
    });
  });
  await page.route('**/api/v1/online/youtube/home', (route) => {
    feedRequested = true;
    return route.fulfill({
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
    });
  });
  try {
    await page.goto('/');
    await expect(page.locator('[data-page-content]')).toHaveAttribute(
      'data-page-phase',
      'idle',
    );
    await expect(
      page.getByRole('status', {
        name: 'Loading YouTube subscriptions',
        exact: true,
      }),
    ).toBeVisible();
    await expect.poll(() => feedRequested).toBe(true);
    account.release();
    await expect(
      page.getByRole('heading', { name: 'Ready upload', exact: true }),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    account.release();
  }
});

test('Home stops reading the YouTube feed once it found the account disconnected', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Home' });
  let accounts = 0;
  let feeds = 0;
  await page.route('**/api/v1/online/providers', (route) =>
    route.fulfill({ json: { youtube: true, twitch: false, kick: false } }),
  );
  await page.route('**/api/v1/online/youtube', async (route) => {
    accounts++;
    await route.fallback();
  });
  await page.route('**/api/v1/online/youtube/home', async (route) => {
    feeds++;
    await route.fallback();
  });
  await page.goto('/');
  await expect.poll(() => accounts).toBe(1);
  await page.getByRole('button', { name: 'Refresh Home' }).click();
  await expect.poll(() => accounts).toBe(2);
  await expect(
    page.getByRole('button', { name: 'Refresh Home' }),
  ).toBeEnabled();
  expect(feeds).toBe(1);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
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

test('Discover appears with its first row while the others load', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
  const gate = responseGate();
  await page.route('**/api/v1/seerr/status', (route) =>
    route.fulfill({ json: { configured: true, ready: true } }),
  );
  await page.route('**/api/v1/seerr/discover/*', async (route) => {
    const feed = new URL(route.request().url()).pathname.split('/').at(-1);
    if (feed !== 'trending') await gate.promise;
    await route.fulfill({
      json: {
        results: [{ id: 1, mediaType: 'movie', title: `First ${feed}` }],
      },
    });
  });
  try {
    await page.goto('/');
    await expect(page.locator('[data-page-content]')).toHaveAttribute(
      'data-page-phase',
      'idle',
    );
    await expect(
      page.getByRole('button', { name: /First trending/ }).first(),
    ).toBeVisible();
    await expect(
      page.getByRole('status', { name: 'Loading Popular movies', exact: true }),
    ).toBeVisible();
    gate.release();
    await expect(
      page.getByRole('button', { name: /First movies/ }),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    gate.release();
  }
});

test('Discover loads a row once it nears the screen', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  const fixture = await installUiFixture(page, { section: 'Discover' });
  const requested: string[] = [];
  await page.route('**/api/v1/seerr/status', (route) =>
    route.fulfill({ json: { configured: true, ready: true } }),
  );
  await page.route('**/api/v1/seerr/discover/*', async (route) => {
    const feed = new URL(route.request().url()).pathname.split('/').at(-1)!;
    requested.push(feed);
    await route.fulfill({
      json: {
        results: Array.from({ length: 8 }, (_, index) => ({
          id: index + 1,
          mediaType: 'movie',
          title: `${feed} ${index}`,
        })),
      },
    });
  });
  await page.goto('/');
  await expect(
    page.getByRole('button', { name: /trending 0/ }).first(),
  ).toBeVisible();
  await page.waitForTimeout(500);
  expect(requested).toContain('trending');
  expect(requested).not.toContain('upcoming-tv');
  const skeleton = page.getByRole('status', {
    name: 'Loading Upcoming series',
    exact: true,
  });
  await skeleton.scrollIntoViewIfNeeded();
  await expect(
    page.getByRole('button', { name: /upcoming-tv 0/ }),
  ).toBeVisible();
  expect(requested.filter((feed) => feed === 'upcoming-tv')).toHaveLength(1);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
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

for (const startup of [
  {
    name: 'a restored session',
    section: 'Home',
    read: '**/api/v1/online/providers',
    event: 'online.configuration.changed',
  },
  {
    name: 'a sign-in',
    section: 'Movies',
    read: '**/api/v1/catalog?**',
    event: 'catalog.changed',
  },
] as const) {
  test(`${startup.name} reads its pages after capturing the event cursor`, async ({
    page,
  }) => {
    const fixture = await installUiFixture(page, {
      section: startup.section,
      signedIn: startup.name === 'a restored session',
    });
    const ticket = responseGate();
    let requested = false;
    let issued = false;
    let early = 0;
    let reads = 0;
    let socket: WebSocketRoute | undefined;
    await page.routeWebSocket(/\/api\/v1\/events(?:\?|$)/, (route) => {
      socket = route;
    });
    await page.route('**/api/v1/auth/event-ticket', async (route) => {
      requested = true;
      await ticket.promise;
      issued = true;
      await route.fulfill({
        json: {
          ticket: 'startup',
          cursor: 41,
          epoch: 'startup',
          version: current,
        },
      });
    });
    await page.route(startup.read, async (route) => {
      reads++;
      if (!issued) early++;
      await route.fallback();
    });
    try {
      await page.goto('/');
      if (startup.name === 'a sign-in') {
        await page.getByLabel('Username', { exact: true }).fill('admin');
        await page
          .getByLabel('Password', { exact: true })
          .fill('local test passphrase');
        await page
          .getByRole('button', { name: 'Sign in', exact: true })
          .click();
      }
      await expect.poll(() => requested).toBe(true);
      ticket.release();
      await expect.poll(() => socket?.url() ?? '').toContain('since=41');
      await expect.poll(() => reads).toBeGreaterThan(0);
      // The connection replays what those reads missed, so nothing reloads.
      await page.waitForTimeout(500);
      expect(reads).toBe(1);
      socket!.send(
        JSON.stringify({ id: 42, kind: startup.event, payload: {} }),
      );
      await expect.poll(() => reads).toBe(2);
      expect(early).toBe(0);
      expect(fixture.errors).toEqual([]);
    } finally {
      ticket.release();
    }
  });
}

test('a restored session reads the server and the session together and loads its page code meanwhile', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Playlists' });
  const health = responseGate();
  const ticket = responseGate();
  const asked = new Set<string>();
  const code: string[] = [];
  page.on('request', (request) => {
    const path = new URL(request.url()).pathname;
    if (path.startsWith('/assets/')) code.push(path);
  });
  await page.route('**/api/v1/health', async (route) => {
    await health.promise;
    await route.fallback();
  });
  for (const path of ['setup', 'auth/methods', 'auth/me'])
    await page.route(`**/api/v1/${path}`, async (route) => {
      asked.add(path);
      await route.fallback();
    });
  await page.route('**/api/v1/auth/event-ticket', async (route) => {
    await ticket.promise;
    await route.fallback();
  });
  await page.route('**/api/v1/playlists', (route) =>
    route.fulfill({ json: { items: [] } }),
  );
  try {
    await page.goto('/');
    await expect.poll(() => asked.size).toBe(3);
    health.release();
    // The page's code downloads while the session's data waits.
    await expect
      .poll(() => code.some((path) => /\/Playlists-[\w-]+\.js$/.test(path)))
      .toBe(true);
    ticket.release();
    await expect(
      page.getByRole('button', { name: 'New playlist', exact: true }),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    health.release();
    ticket.release();
  }
});

test('an appearance change delivered during startup outlasts the older first read', async ({
  page,
}) => {
  await installUiFixture(page, { section: 'Home' });
  const read = responseGate();
  let socket: WebSocketRoute | undefined;
  let reads = 0;
  let answered = false;
  await page.routeWebSocket(/\/api\/v1\/events(?:\?|$)/, (route) => {
    socket = route;
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({
      json: {
        ticket: 'startup',
        cursor: 41,
        epoch: 'startup',
        version: current,
      },
    }),
  );
  await page.route('**/api/v1/me/appearance', async (route) => {
    if (++reads > 1) return route.fallback();
    await read.promise;
    await route.fulfill({ json: { theme: 'light' } });
    answered = true;
  });
  try {
    await page.goto('/');
    await expect.poll(() => reads).toBeGreaterThan(0);
    await expect.poll(() => Boolean(socket)).toBe(true);
    socket!.send(
      JSON.stringify({
        id: 42,
        kind: 'appearance.changed',
        payload: { appearance: { theme: 'dark' } },
      }),
    );
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    read.release();
    await expect.poll(() => answered).toBe(true);
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  } finally {
    read.release();
  }
});
