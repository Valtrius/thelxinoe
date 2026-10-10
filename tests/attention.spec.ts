import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('delayed request details cannot acknowledge a newer unseen decision', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
  const entry = {
    id: 'seerr:service:7',
    revision: 'denied:2:[]',
    target: 'requests',
    resource: '7',
    severity: 'warning',
    message: 'Request declined',
    dismissible: true,
    media_ids: [],
  };
  let denied = false;
  let fresh = false;
  let seen = false;
  const releases: (() => void)[] = [];
  const acknowledged: unknown[] = [];
  await page.route('**/api/v1/me/attention**', async (route) => {
    if (route.request().method() === 'PUT') {
      acknowledged.push(route.request().postDataJSON());
      seen = true;
      return route.fulfill({ json: { saved: true } });
    }
    return route.fulfill({ json: { items: denied && !seen ? [entry] : [] } });
  });
  await page.route('**/api/v1/seerr/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/status'))
      return route.fulfill({ json: { configured: true, ready: true } });
    if (path.endsWith('/requests'))
      return route.fulfill({
        json: {
          pageInfo: { pages: 1 },
          results: [
            {
              id: 7,
              type: 'movie',
              status: fresh ? 3 : 1,
              media: { tmdbId: 11, mediaType: 'movie', status: 2 },
              attention: {
                id: entry.id,
                revision: fresh ? entry.revision : 'pending:1:[]',
              },
            },
          ],
        },
      });
    if (path.endsWith('/movie/11')) {
      if (!seen) await new Promise<void>((resolve) => releases.push(resolve));
      return route.fulfill({
        json: { id: 11, title: 'Delayed request', mediaType: 'movie' },
      });
    }
    return route.fulfill({ json: { results: [], page: 1, totalPages: 1 } });
  });
  await page.goto('/');
  const requests = page
    .getByRole('navigation', { name: 'Discover navigation' })
    .getByRole('button', { name: /^My requests/ });
  await requests.click();
  await expect.poll(() => releases.length).toBe(1);
  denied = true;
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect(requests.locator('[data-attention-severity]')).toBeVisible();
  const response = page.waitForResponse('**/api/v1/seerr/movie/11');
  releases[0]();
  await (await response).finished();
  // The replacement snapshot's details are still withheld. Its decision cannot
  // have been displayed, even if the older detail request has just completed.
  await expect.poll(() => releases.length).toBe(2);
  expect(acknowledged).toEqual([]);
  await expect(requests.locator('[data-attention-severity]')).toBeVisible();
  releases[1]();
  await expect(
    page.getByRole('button', {
      name: 'Delayed request Pending approval',
      exact: true,
    }),
  ).toBeVisible();
  expect(acknowledged).toEqual([]);
  await expect(requests.locator('[data-attention-severity]')).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('stale-seerr-snapshot-stays-unread.png'),
  });
  fresh = true;
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect.poll(() => releases.length).toBe(3);
  releases[2]();
  await expect(
    page.getByRole('button', { name: 'Delayed request Declined', exact: true }),
  ).toBeVisible();
  await expect(requests.locator('[data-attention-severity]')).toHaveCount(0);
  expect(acknowledged).toEqual([{ revision: entry.revision }]);
  await page.screenshot({
    path: testInfo.outputPath('displayed-request-decision.png'),
  });
  // Subsequent refreshes caused by the successful acknowledgment may load again.
  releases.slice(3).forEach((release) => release());
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('native request history keeps newer decisions unread until refreshed and reaches older pages', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
  let newer = false;
  let release: (() => void) | undefined;
  const seen = new Set<string>();
  const acknowledged: unknown[] = [];
  const entries = [
    {
      id: 'request:recent',
      revision: 'g:denied:2',
      target: 'requests',
      resource: 'recent',
      severity: 'warning',
      message: 'Request declined',
      dismissible: true,
      media_ids: [],
    },
    {
      id: 'request:older',
      revision: 'g:failed:1',
      target: 'requests',
      resource: 'older',
      severity: 'error',
      message: 'Request failed',
      dismissible: true,
      media_ids: [],
    },
  ];
  await page.route('**/api/v1/seerr/**', (route) =>
    route.fulfill({
      json: new URL(route.request().url()).pathname.endsWith('/status')
        ? { configured: false, ready: false }
        : { results: [], pageInfo: { pages: 1 } },
    }),
  );
  await page.route('**/api/v1/me/attention**', (route) => {
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      acknowledged.push(body);
      seen.add(body.revision);
      return route.fulfill({ json: { saved: true } });
    }
    return route.fulfill({
      json: {
        items: entries.filter(
          (entry) =>
            (entry.resource === 'older' || newer) && !seen.has(entry.revision),
        ),
      },
    });
  });
  await page.route('**/api/v1/acquisition/requests**', async (route) => {
    const currentPage = Number(
      new URL(route.request().url()).searchParams.get('page') || 1,
    );
    if (newer && !release)
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    const entry = entries[currentPage === 1 ? 0 : 1];
    return route.fulfill({
      json: {
        page: currentPage,
        pages: 2,
        items: [
          {
            id: entry.resource,
            title: currentPage === 1 ? 'Recent album' : 'Older album',
            kind: 'lidarr',
            state:
              currentPage === 1 ? (newer ? 'denied' : 'pending') : 'failed',
            attention: {
              id: entry.id,
              revision:
                currentPage === 1 && !newer ? 'g:pending:1' : entry.revision,
            },
          },
        ],
      },
    });
  });
  await page.goto('/');
  const requests = page
    .getByRole('navigation', { name: 'Discover navigation' })
    .getByRole('button', { name: /^My requests/ });
  await requests.click();
  const pending = page.getByRole('button', {
    name: 'Recent album pending',
    exact: true,
  });
  await expect(pending).toBeVisible();
  newer = true;
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect.poll(() => Boolean(release)).toBe(true);
  await pending.hover();
  expect(acknowledged).toEqual([]);
  await page.screenshot({
    path: testInfo.outputPath('stale-history-stays-unread.png'),
  });
  release!();
  await page.mouse.move(0, 0);
  await page.getByRole('button', { name: /^Recent album denied/ }).hover();
  await expect.poll(() => acknowledged.length).toBe(1);
  await expect(requests.locator('[data-attention-severity]')).toBeVisible();
  await page
    .getByRole('button', { name: 'Older requests', exact: true })
    .click();
  await page.getByRole('button', { name: /^Older album failed/ }).focus();
  await expect(requests.locator('[data-attention-severity]')).toHaveCount(0);
  expect(acknowledged).toEqual(
    entries.map((entry) => ({ revision: entry.revision })),
  );
  await page
    .getByRole('button', { name: 'Newer requests', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Recent album denied', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('older-request-acknowledged.png'),
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page
    .getByRole('button', { name: 'Older requests', exact: true })
    .scrollIntoViewIfNeeded();
  await page.screenshot({
    path: testInfo.outputPath('request-pages-mobile.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('menu dots track live severity while pages are closed and survive sidebar collapse', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    section: 'Movies',
  });
  await page.route('**/api/v1/catalog/roots', (route) =>
    route.fulfill({ json: { items: [], media_mount: '/media' } }),
  );
  let items = [
    {
      id: 'service:radarr',
      severity: 'error',
      target: 'services',
      resource: 'radarr',
      message: 'Radarr unavailable',
      revision: '1',
      dismissible: false,
      media_ids: [],
    },
    {
      id: 'account:youtube',
      severity: 'warning',
      target: 'online',
      resource: 'youtube',
      message: 'Reconnect YouTube',
      revision: '1',
      dismissible: false,
      media_ids: [],
    },
    {
      id: 'product-release',
      severity: 'info',
      target: 'server',
      resource: null,
      message: 'Server update available',
      revision: '1',
      dismissible: false,
      media_ids: [],
    },
  ];
  await page.route('**/api/v1/me/attention', (route) =>
    route.fulfill({ json: { items } }),
  );
  let sendEvent: ((kind: string) => void) | undefined;
  let eventId = 0;
  await page.routeWebSocket(/\/api\/v1\/events(?:\?|$)/, (socket) => {
    sendEvent = (kind) =>
      socket.send(JSON.stringify({ id: ++eventId, kind, payload: {} }));
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({
      json: { ticket: 'layout', cursor: 0, epoch: 'layout', version: '0.1.0' },
    }),
  );
  await page.goto('/');
  await expect.poll(() => Boolean(sendEvent)).toBe(true);
  const settings = page.getByRole('link', { name: 'Settings', exact: true });
  const dot = settings.locator('[data-attention-severity]');
  await expect(
    page.getByRole('button', { name: 'Notifications', exact: true }),
  ).toHaveCount(0);
  await expect(dot).toHaveAttribute('data-attention-severity', 'error');
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(dot).toBeVisible();
  await page.getByRole('button', { name: 'Expand sidebar' }).click();
  await settings.click();
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await expect(
    nav
      .getByRole('link', { name: 'Media services', exact: true })
      .locator('[data-attention-severity]'),
  ).toHaveAttribute('data-attention-severity', 'error');
  await nav.getByRole('link', { name: 'Media services', exact: true }).click();
  await expect(dot).toHaveAttribute('data-attention-severity', 'error');
  await expect(
    page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('link', { name: 'Radarr', exact: true })
      .locator('[data-attention-severity]'),
  ).toHaveAttribute('data-attention-severity', 'error');
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  for (const severity of ['warning', 'info', null]) {
    items = items.slice(1);
    await page.evaluate(() =>
      window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
    );
    if (severity)
      await expect(dot).toHaveAttribute('data-attention-severity', severity);
    else await expect(dot).toHaveCount(0);
  }
  items = [
    {
      id: 'tools-release:yt-dlp',
      severity: 'info',
      target: 'server',
      resource: 'yt-dlp',
      message: 'yt-dlp update available',
      revision: 'candidate-2',
      dismissible: false,
      media_ids: [],
    },
  ];
  sendEvent!('attention.changed');
  await expect(dot).toHaveAttribute('data-attention-severity', 'info');
  await settings.click();
  const server = nav.getByRole('link', { name: 'Server', exact: true });
  await expect(server.locator('[data-attention-severity]')).toHaveAttribute(
    'data-attention-severity',
    'info',
  );
  await server.click();
  await expect(
    page.getByRole('region', { name: 'Server tools' }),
  ).toBeVisible();
  await expect(dot).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('server-tool-update-dot.png'),
  });
  items = [];
  sendEvent!('attention.changed');
  await expect(dot).toHaveCount(0);
  await expect(server.locator('[data-attention-severity]')).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('resolved-menu-dots.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const [section, kind] of [
  ['Movies', 'movie'],
  ['Shows', 'show'],
  ['Music', 'artist'],
]) {
  test(`available ${section} requests mark their posters and acknowledge only on hover`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: 'Discover' });
    await page.route('**/api/v1/seerr/status', (route) =>
      route.fulfill({ json: { configured: false, ready: false } }),
    );
    let seen = false;
    let fail = true;
    const acknowledgments: unknown[] = [];
    const entry = {
      id: 'request:requested',
      severity: 'info',
      target: section.toLowerCase(),
      resource: 'requested',
      message: 'Requested media is available',
      revision: 'available:2',
      dismissible: true,
      media_ids: ['requested', 'album'],
    };
    await page.route('**/api/v1/me/attention**', async (route) => {
      if (route.request().method() === 'PUT') {
        acknowledgments.push(route.request().postDataJSON());
        if (fail)
          return route.fulfill({
            status: 503,
            json: { error: { message: 'Temporary failure' } },
          });
        seen = true;
        return route.fulfill({ json: { saved: true } });
      }
      return route.fulfill({ json: { items: seen ? [] : [entry] } });
    });
    await page.route('**/api/v1/catalog?**', (route) =>
      route.fulfill({
        json: {
          items: new URL(route.request().url()).searchParams.has('parent')
            ? [
                {
                  id: 'album',
                  kind: 'album',
                  title: 'Requested album',
                  available: true,
                  metadata: {},
                },
              ]
            : [
                {
                  id: 'other',
                  kind,
                  title: 'Other media',
                  available: true,
                  metadata: {},
                },
                {
                  id: 'requested',
                  kind,
                  title: 'Requested media',
                  available: true,
                  metadata: {},
                },
              ],
        },
      }),
    );
    if (section === 'Music')
      await page.route('**/api/v1/catalog/requested', (route) =>
        route.fulfill({
          json: {
            id: 'requested',
            kind,
            title: 'Requested media',
            available: true,
            metadata: {},
          },
        }),
      );
    await page.goto('/');
    const menu = page
      .getByRole('complementary', { name: 'Application sidebar' })
      .getByRole('link', { name: section, exact: true });
    await expect(menu.locator('[data-attention-severity]')).toBeVisible();
    await expect(
      page
        .getByRole('link', { name: 'Home', exact: true })
        .locator('[data-attention-severity]'),
    ).toHaveCount(0);
    await menu.click();
    await page.mouse.move(0, 0);
    let poster = page.locator('[data-layout-key="library:requested"]');
    await expect(poster.locator('[data-attention-severity]')).toBeVisible();
    await expect(
      poster.getByRole('button', {
        name: `Requested media ${kind}`,
        exact: true,
      }),
    ).toHaveAccessibleDescription('Info: Requested media is available');
    await page.locator('[data-layout-key="library:other"] .tile-art').hover();
    expect(acknowledgments).toEqual([]);
    if (section === 'Music') {
      await poster.locator('.tile-art').hover();
      expect(acknowledgments).toEqual([]);
      await poster.locator('.card-primary').click();
      poster = page.locator('[data-layout-key="library:album"]');
      await expect(poster.locator('[data-attention-severity]')).toBeVisible();
    }
    await page.screenshot({
      path: testInfo.outputPath('requested-poster-dot.png'),
    });
    await poster.locator('.tile-art').hover();
    await expect.poll(() => acknowledgments.length).toBe(1);
    await expect(poster.locator('[data-attention-severity]')).toBeVisible();
    await expect(poster.locator('.card-primary')).toHaveAccessibleDescription(
      'Could not clear this marker. Hover again to retry.',
    );
    await page.mouse.move(0, 0);
    fail = false;
    await poster.locator('.tile-art').hover();
    await expect(poster.locator('[data-attention-severity]')).toHaveCount(0);
    await expect(
      poster.getByRole('button', {
        name:
          section === 'Music'
            ? 'Requested album album'
            : `Requested media ${kind}`,
        exact: true,
      }),
    ).toHaveAccessibleDescription('');
    await expect(menu.locator('[data-attention-severity]')).toHaveCount(0);
    expect(acknowledgments).toEqual([
      { revision: entry.revision },
      { revision: entry.revision },
    ]);
    await page.reload();
    await expect(menu.locator('[data-attention-severity]')).toHaveCount(0);
    await page.screenshot({
      path: testInfo.outputPath('acknowledged-request.png'),
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('a newer available request remains marked when an older hover acknowledgment finishes', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  let revision = 'available:1';
  let release: (() => void) | undefined;
  let markStarted: () => void;
  const started = new Promise<void>((resolve) => {
    markStarted = resolve;
  });
  await page.route('**/api/v1/me/attention**', async (route) => {
    if (route.request().method() === 'PUT') {
      markStarted();
      await new Promise<void>((done) => {
        release = done;
      });
      return route.fulfill({ json: { saved: true } });
    }
    return route.fulfill({
      json: {
        items: [
          {
            id: 'request:movie',
            severity: 'info',
            target: 'movies',
            resource: 'movie',
            revision,
            dismissible: true,
            message: `Requested media is available ${revision}`,
            media_ids: ['movie-0'],
          },
        ],
      },
    });
  });
  await page.goto('/');
  const poster = page.locator('[data-layout-key="library:movie-0"]');
  await expect(poster.locator('[data-attention-severity]')).toBeVisible();
  await poster.locator('.tile-art').hover();
  await started;
  revision = 'available:2';
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect(poster.locator('[data-attention-severity]')).toHaveAttribute(
    'aria-label',
    /available:2/,
  );
  const acknowledged = page.waitForResponse(
    (response) =>
      response.request().method() === 'PUT' &&
      response.url().includes('/me/attention/'),
  );
  release!();
  await (await acknowledged).finished();
  await expect(poster.locator('[data-attention-severity]')).toHaveAttribute(
    'aria-label',
    /available:2/,
  );
  await page.screenshot({
    path: testInfo.outputPath('newer-request-remains-marked.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const [type, section] of [
  ['movie', 'Movies'],
  ['tv', 'Shows'],
]) {
  test(`available ${type} requests open ${section} from the request list`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: 'Discover' });
    await page.route('**/api/v1/seerr/**', (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/status'))
        return route.fulfill({ json: { configured: true, ready: true } });
      if (path.endsWith('/requests'))
        return route.fulfill({
          json: {
            pageInfo: { pages: 1 },
            results: [
              {
                id: 1,
                status: 5,
                type,
                media: { tmdbId: 11, mediaType: type, status: 5 },
              },
            ],
          },
        });
      if (path.endsWith('/11'))
        return route.fulfill({
          json: { id: 11, title: 'Available request', mediaType: type },
        });
      return route.fulfill({ json: { results: [], page: 1, totalPages: 1 } });
    });
    await page.goto('/');
    await page
      .getByRole('navigation', { name: 'Discover navigation' })
      .getByRole('button', { name: 'My requests', exact: true })
      .click();
    await page
      .getByRole('button', { name: 'Available request Available', exact: true })
      .click();
    await expect(
      page.getByRole('heading', { name: section, exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath('available-request-library.png'),
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}
