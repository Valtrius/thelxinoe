import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

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
  const settings = page.getByRole('button', { name: 'Settings', exact: true });
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
      .getByRole('button', { name: 'Media services', exact: true })
      .locator('[data-attention-severity]'),
  ).toHaveAttribute('data-attention-severity', 'error');
  await nav
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  await expect(dot).toHaveAttribute('data-attention-severity', 'error');
  await expect(
    page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('button', { name: 'Radarr', exact: true })
      .locator('[data-attention-severity]'),
  ).toHaveAttribute('data-attention-severity', 'error');
  await page.getByRole('button', { name: 'Movies', exact: true }).click();
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
  sendEvent!('tools.changed');
  await expect(dot).toHaveAttribute('data-attention-severity', 'info');
  await settings.click();
  const server = nav.getByRole('button', { name: 'Server', exact: true });
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
  sendEvent!('tools.changed');
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
    const fixture = await installUiFixture(page, { section: 'Home' });
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
    await page.goto('/');
    const menu = page
      .getByRole('complementary', { name: 'Application sidebar' })
      .getByRole('button', { name: section, exact: true });
    await expect(menu.locator('[data-attention-severity]')).toBeVisible();
    await expect(
      page
        .getByRole('button', { name: 'Home', exact: true })
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
    const fixture = await installUiFixture(page, { section: 'Home' });
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
