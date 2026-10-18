import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('activity arcs follow operation lifetime, scope, severity and reduced motion across navigation', async ({
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
      message: 'Radarr API unavailable',
      revision: '1',
      dismissible: false,
      media_ids: [],
    },
  ];
  let activity = [
    {
      id: 'setup:radarr',
      target: 'services',
      resource: 'radarr',
      state: 'running',
      message: 'Restarting Radarr',
      media_ids: [],
    },
  ];
  let offline = false;
  await page.route('**/api/v1/me/attention', (route) =>
    offline
      ? route.fulfill({
          status: 503,
          json: { error: { code: 'unavailable', message: 'Unavailable' } },
        })
      : route.fulfill({ json: { items, activity } }),
  );
  await page.goto('/');
  const settings = page.getByRole('link', { name: 'Settings', exact: true });
  const marker = settings.locator('[data-activity-state]');
  await expect(marker).toHaveAttribute('data-activity-state', 'running');
  await expect(marker).toHaveAttribute('data-attention-severity', 'error');
  const icon = await settings.locator('svg').first().boundingBox();
  const placement = await marker.boundingBox();
  expect(placement!.x).toBeGreaterThan(icon!.x);
  expect(placement!.y).toBeLessThan(icon!.y);
  await expect(marker).toHaveAttribute(
    'aria-label',
    /Restarting Radarr.*\nIssue: Radarr API unavailable/,
  );
  await expect
    .poll(() =>
      marker.evaluate((node) => node.getAnimations({ subtree: true }).length),
    )
    .toBeGreaterThan(0);
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(marker).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect
    .poll(() =>
      marker.evaluate((node) => node.getAnimations({ subtree: true }).length),
    )
    .toBe(0);
  await page.screenshot({
    path: testInfo.outputPath(
      'activity-with-issue-collapsed-reduced-motion.png',
    ),
  });
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await settings.click();
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await expect(
    nav
      .getByRole('link', { name: 'Media services', exact: true })
      .locator('[data-activity-state]'),
  ).toHaveAttribute('data-activity-state', 'running');
  await expect(
    nav
      .getByRole('link', { name: 'Server', exact: true })
      .locator('[data-activity-state]'),
  ).toHaveCount(0);
  await nav.getByRole('link', { name: 'Media services', exact: true }).click();
  const services = page.getByRole('navigation', { name: 'Select service' });
  const radarr = services
    .getByRole('link', { name: 'Radarr', exact: true })
    .locator('[data-activity-state]');
  await expect(radarr).toHaveAttribute('data-activity-state', 'running');
  await expect(
    services
      .getByRole('link', { name: 'Sonarr', exact: true })
      .locator('[data-activity-state]'),
  ).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('activity-scoped-service.png'),
  });
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  const refresh = () =>
    page.evaluate(() =>
      window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
    );
  activity = activity.map((item) => ({
    ...item,
    state: 'queued',
    message: 'Radarr restart queued',
  }));
  await refresh();
  await expect(marker).toHaveAttribute('data-activity-state', 'queued');
  await expect
    .poll(() =>
      marker.evaluate((node) => node.getAnimations({ subtree: true }).length),
    )
    .toBe(0);
  activity = activity.map((item) => ({
    ...item,
    state: 'running',
    message: 'Restarting Radarr',
  }));
  await refresh();
  await expect(marker).toHaveAttribute('data-activity-state', 'running');
  offline = true;
  await refresh();
  await expect(marker).toHaveAttribute('data-activity-state', 'unknown');
  await expect(marker).toHaveAttribute('aria-label', /Status unknown/);
  await expect
    .poll(() =>
      marker.evaluate((node) => node.getAnimations({ subtree: true }).length),
    )
    .toBe(0);
  offline = false;
  activity = [];
  await refresh();
  await expect(marker).toHaveCount(0);
  await expect(settings.locator('[data-attention-severity]')).toHaveAttribute(
    'data-attention-severity',
    'error',
  );
  items = [];
  await refresh();
  await expect(settings.locator('[data-attention-severity]')).toHaveCount(0);
  activity = [
    {
      id: 'backup:1',
      target: 'backups',
      resource: 'backup-1',
      state: 'running',
      message: 'Creating encrypted backup',
      media_ids: [],
    },
  ];
  await refresh();
  await expect(marker).toHaveAttribute('data-activity-state', 'running');
  await expect(settings.locator('[data-attention-severity]')).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await expect(
    page
      .getByRole('dialog')
      .getByRole('link', { name: 'Settings', exact: true })
      .locator('[data-activity-state]'),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('activity-mobile-navigation.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('a pending service action remains marked after leaving its page and hands off to durable activity', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.route('**/api/v1/catalog/roots', (route) =>
    route.fulfill({ json: { items: [], media_mount: '/media' } }),
  );
  let release: (() => void) | undefined;
  let activity: {
    id: string;
    target: string;
    resource: string;
    state: string;
    message: string;
    media_ids: string[];
  }[] = [];
  let holdSnapshot = false;
  let releaseSnapshot: (() => void) | undefined;
  await page.route('**/api/v1/me/attention', async (route) => {
    const snapshot = structuredClone(activity);
    if (holdSnapshot) {
      holdSnapshot = false;
      await new Promise<void>((resolve) => {
        releaseSnapshot = resolve;
      });
    }
    return route.fulfill({ json: { items: [], activity: snapshot } });
  });
  await page.route(
    '**/api/v1/admin/stack/managed-radarr/action',
    async (route) => {
      await new Promise<void>((resolve) => {
        release = resolve;
      });
      activity = [
        {
          id: 'restart:radarr',
          target: 'services',
          resource: 'radarr',
          state: 'running',
          message: 'Restarting Radarr',
          media_ids: [],
        },
      ];
      return route.fulfill({ json: { accepted: true } });
    },
  );
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('link', { name: 'Radarr', exact: true })
    .click();
  await page.getByRole('button', { name: 'Restart', exact: true }).click();
  await expect.poll(() => Boolean(release)).toBe(true);
  const settings = page.getByRole('link', { name: 'Settings', exact: true });
  await expect(settings.locator('[data-activity-state]')).toHaveAttribute(
    'data-activity-state',
    'running',
  );
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(settings.locator('[data-activity-state]')).toHaveAttribute(
    'data-activity-state',
    'running',
  );
  holdSnapshot = true;
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect.poll(() => Boolean(releaseSnapshot)).toBe(true);
  const accepted = page.waitForResponse((response) =>
    response.url().endsWith('/admin/stack/managed-radarr/action'),
  );
  release!();
  await accepted;
  await page.evaluate(async () => {
    for (let frame = 0; frame < 6; frame++)
      await new Promise(requestAnimationFrame);
  });
  await expect(settings.locator('[data-activity-state]')).toHaveAttribute(
    'data-activity-state',
    'running',
  );
  await page.screenshot({
    path: testInfo.outputPath('service-action-awaits-fresh-snapshot.png'),
  });
  releaseSnapshot!();
  await expect(settings.locator('[data-activity-state]')).toHaveAttribute(
    'aria-label',
    /Restarting Radarr/,
  );
  await page.screenshot({
    path: testInfo.outputPath('service-action-persists-after-navigation.png'),
  });
  activity = [];
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect(settings.locator('[data-activity-state]')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

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

test('opening Requests marks decisions on native acquisition requests as seen', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Discover' });
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
        items: entries.filter((entry) => !seen.has(entry.revision)),
      },
    });
  });
  await page.goto('/');
  const requests = page
    .getByRole('navigation', { name: 'Discover navigation' })
    .getByRole('button', { name: /^My requests/ });
  await expect(requests.locator('[data-attention-severity]')).toBeVisible();
  expect(acknowledged).toEqual([]);
  await requests.click();
  await expect.poll(() => acknowledged.length).toBe(2);
  expect(acknowledged).toEqual(
    expect.arrayContaining(
      entries.map((entry) => ({ revision: entry.revision })),
    ),
  );
  await page.evaluate(() =>
    window.dispatchEvent(new Event('thelxinoe-attention-refresh')),
  );
  await expect(requests.locator('[data-attention-severity]')).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('native-requests-seen.png'),
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
  test(`available ${type} requests open their title from the request list`, async ({
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
      page.getByRole('heading', {
        name: 'Available request',
        level: 1,
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page
        .getByRole('article', { name: 'Available request' })
        .getByText(new RegExp(`^${section === 'Movies' ? 'Movie' : 'Series'}`)),
    ).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Back to discover', exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath('available-request-title.png'),
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}
