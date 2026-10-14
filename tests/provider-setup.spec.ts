import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

async function providerFixture(page: Page, platform: string, admin = true) {
  const fixture = await installUiFixture(page, {
    section: platform,
    role: admin ? 'admin' : 'user',
  });
  const state = {
    providers: { youtube: true, twitch: true, kick: true },
    failToggle: false,
    configured: false,
    linkingAvailable: true,
    reconnect: false,
    pending: false,
    authPhase: 'waiting' as 'waiting' | 'connecting' | 'failed',
    authError: null as string | null,
    authExpiresAt: Math.floor(Date.now() / 1000) + 600,
    connected: false,
    sync: null as {
      last_complete: number | null;
      next_run: number;
      error: string | null;
      in_progress: boolean;
    } | null,
    twitchItems: [] as {
      id: string;
      login: string;
      display_name: string;
      title: string;
      category: string;
      viewers: number;
      started_at: string;
    }[],
    kickConnected: false,
    channels: [] as {
      slug: string;
      title: string;
      live: null;
      updated_at: number;
      viewers: number;
      category: string;
    }[],
    writes: [] as { path: string; body: unknown }[],
    adminReads: [] as string[],
  };
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace('/api/v1', '');
    const json = (value: unknown) => route.fulfill({ json: value });
    if (path === '/online/providers') return json(state.providers);
    if (path.startsWith('/admin/online/providers/')) {
      if (state.failToggle)
        return route.fulfill({
          status: 503,
          json: { error: { message: 'Settings unavailable' } },
        });
      const provider = path.split('/').at(-1) as keyof typeof state.providers;
      state.providers[provider] = request.postDataJSON().enabled;
      return json(state.providers);
    }
    if (path.startsWith('/admin/online')) {
      if (request.method() === 'PUT') {
        state.configured = true;
        state.writes.push({ path, body: request.postDataJSON() });
        return json({ saved: true });
      }
      state.adminReads.push(path);
      return json({
        configured: state.configured,
        google_configured: state.configured,
        redirect_uri: state.linkingAvailable
          ? 'https://media.test/api/v1/online/youtube/callback'
          : null,
        youtube_downloads: false,
        youtube_daily_quota: 10000,
        youtube_network: 'auto',
        youtube_network_status: { ipv4: true, ipv6: true, current: 'ipv4' },
        quota: { used: 0, blocked: false },
      });
    }
    if (path === '/online/twitch/connect') {
      state.pending = true;
      state.writes.push({ path, body: request.postDataJSON() });
      return json({ started: true });
    }
    if (path.endsWith('/authorization') && request.method() === 'DELETE') {
      state.pending = false;
      return json({ cancelled: true });
    }
    if (path === '/online/twitch' || path === '/online/youtube')
      return json({
        configured: state.configured,
        linking_available: state.linkingAvailable,
        account: {
          status: state.connected
            ? 'connected'
            : state.reconnect
              ? 'reconnect_required'
              : 'disconnected',
          display_name: '',
        },
        pending: state.pending
          ? {
              phase: state.authPhase,
              error: state.authError,
              ...(path.endsWith('twitch')
                ? {
                    user_code: 'TEST-CODE',
                    verification_uri: 'https://www.twitch.tv/activate',
                  }
                : {}),
              expires_at: state.authExpiresAt,
            }
          : null,
        sync: state.sync,
      });
    if (path === '/online/youtube/watchlists') return json([]);
    if (path === '/online/twitch/feed')
      return json({ items: state.twitchItems });
    if (path === '/online/youtube/feed' || path === '/online/youtube/browse')
      return json({
        items: [],
        channels: [],
        counts: {
          all: 0,
          unwatched: 0,
          inProgress: 0,
          watched: 0,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 0,
        },
        page: 0,
        hasMore: false,
      });
    if (path === '/online/kick')
      return json({
        configured: state.configured,
        connected: state.kickConnected,
        items: state.channels,
      });
    if (path === '/online/kick/channels') {
      state.writes.push({ path, body: request.postDataJSON() });
      state.channels.push({
        slug: 'fixture',
        title: '',
        live: null,
        updated_at: 0,
        viewers: 0,
        category: '',
      });
      state.kickConnected = true;
      return json({ slug: 'fixture' });
    }
    if (path === '/online/kick/channels/fixture') {
      state.channels = state.channels.filter(
        (channel) => channel.slug !== 'fixture',
      );
      state.writes.push({ path, body: null });
      return json({ removed: true });
    }
    if (path === '/online/kick/sync') {
      state.writes.push({ path, body: null });
      return json({ started: true });
    }
    if (path === '/online/kick/connect') {
      state.kickConnected = true;
      state.writes.push({ path, body: request.postDataJSON() });
      return json({ connected: true });
    }
    return route.fallback();
  });
  return { ...fixture, state };
}

test('YouTube setup saves on the page and becomes a connection action', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'YouTube');
  await page.goto('/');
  await expect(page.getByLabel('Authorized redirect URI')).toHaveValue(
    'https://media.test/api/v1/online/youtube/callback',
  );
  await expect(page.getByLabel('Application home page')).toHaveValue(
    'https://media.test/about/index.html',
  );
  await expect(page.getByLabel('Application privacy policy link')).toHaveValue(
    'https://media.test/about/privacy.html',
  );
  await expect(
    page.getByRole('link', { name: 'Open home page' }),
  ).toHaveAttribute('href', 'https://media.test/about/index.html');
  await expect(
    page.getByRole('link', { name: 'Open privacy policy' }),
  ).toHaveAttribute('href', 'https://media.test/about/privacy.html');
  await expect(page.getByLabel('Google client ID')).toHaveCSS(
    'border-top-width',
    '1px',
  );
  await expect(
    page.getByRole('link', { name: 'YouTube Data API v3' }),
  ).toHaveAttribute(
    'href',
    'https://console.cloud.google.com/apis/library/youtube.googleapis.com',
  );
  await page.screenshot({ path: '.local/provider-setup/youtube-desktop.png' });
  await page.getByLabel('Google client ID').fill('fixture-client');
  await page.getByLabel('Google client secret').fill('fixture-secret');
  await page.getByRole('button', { name: 'Apply Google application' }).click();
  await expect(
    page.getByRole('button', { name: 'Connect YouTube', exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel('Google client secret')).toHaveCount(0);
  expect(fixture.state.writes[0]).toMatchObject({
    path: '/admin/online',
    body: {
      google: { client_id: 'fixture-client', client_secret: 'fixture-secret' },
    },
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Twitch keeps loading until the first followed snapshot arrives', async ({
  page,
}, testInfo) => {
  const fixture = await providerFixture(page, 'Twitch');
  fixture.state.configured = true;
  fixture.state.connected = true;
  fixture.state.sync = {
    last_complete: null,
    next_run: 0,
    error: null,
    in_progress: true,
  };
  await page.goto('/');
  const loading = page.getByRole('status', { name: 'Loading Twitch streams' });
  await expect(loading).toBeVisible();
  await page.reload();
  await expect(loading).toBeVisible();
  await expect(page.getByText('No followed channels are live')).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath('first-sync-pending.png'),
  });

  fixture.state.twitchItems = [
    {
      id: '42',
      login: 'fixturechannel',
      display_name: 'Fixture channel',
      title: 'Followed channel arrived',
      category: 'Science',
      viewers: 8,
      started_at: '2026-10-12T12:00:00Z',
    },
  ];
  fixture.state.sync = {
    last_complete: Math.floor(Date.now() / 1000),
    next_run: Math.floor(Date.now() / 1000) + 60,
    error: null,
    in_progress: false,
  };
  await expect(page.getByText('Followed channel arrived')).toBeVisible();
  await expect(loading).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('first-sync-ready.png') });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const platform of ['Twitch', 'YouTube']) {
  test(`${platform} sign-in shows live progress through approval, refresh and connection`, async ({
    page,
  }, testInfo) => {
    const fixture = await providerFixture(page, platform);
    fixture.state.configured = true;
    fixture.state.pending = true;
    await page.goto('/');
    const progress = page.getByRole('status').filter({
      hasText:
        platform === 'Twitch'
          ? 'Waiting for Twitch approval'
          : 'Waiting for Google approval',
    });
    await expect(progress).toBeVisible();
    await expect(progress.locator('svg')).toHaveCSS('animation-name', /spin/);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(progress).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({ path: testInfo.outputPath('waiting-mobile.png') });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.reload();
    await expect(progress).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath('waiting-for-approval.png'),
    });

    fixture.state.authPhase = 'connecting';
    await expect(
      page.getByRole('status').filter({ hasText: 'Finishing sign-in' }),
    ).toBeVisible();
    if (platform === 'Twitch') {
      await expect(
        page.getByRole('button', { name: 'Copy Twitch device code' }),
      ).toHaveCount(0);
      await expect(
        page.getByRole('button', { name: 'Open verification page' }),
      ).toHaveCount(0);
    }
    await page.reload();
    await expect(
      page.getByRole('status').filter({ hasText: 'Finishing sign-in' }),
    ).toBeVisible();
    await page.screenshot({
      path: testInfo.outputPath('finishing-sign-in.png'),
    });
    fixture.state.pending = false;
    fixture.state.connected = true;
    await expect(
      page.getByRole('status').filter({ hasText: 'Finishing sign-in' }),
    ).toHaveCount(0);
    await expect(
      page.getByRole('button', { name: `Connect ${platform}`, exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole('button', {
        name: platform === 'Twitch' ? 'Refresh' : 'Refresh YouTube feed',
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('connected.png') });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });

  test(`${platform} sign-in stops waiting when cancelled or expired`, async ({
    page,
  }, testInfo) => {
    const fixture = await providerFixture(page, platform);
    fixture.state.configured = true;
    fixture.state.pending = true;
    await page.clock.install();
    await page.goto('/');
    await expect(
      page.getByRole('status').filter({ hasText: 'Waiting for' }),
    ).toBeVisible();
    await page
      .getByRole('button', {
        name: platform === 'Twitch' ? 'Cancel' : 'Cancel sign-in',
        exact: true,
      })
      .click();
    await expect(
      page.getByRole('button', { name: `Connect ${platform}`, exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('status').filter({ hasText: 'Waiting for' }),
    ).toHaveCount(0);
    fixture.state.pending = true;
    fixture.state.authExpiresAt = Math.floor(Date.now() / 1000) + 5;
    await page.reload();
    await expect(
      page.getByRole('status').filter({ hasText: 'Waiting for' }),
    ).toBeVisible();
    await page.clock.fastForward(6000);
    await expect(
      page.getByRole('alert').filter({ hasText: 'expired' }),
    ).toBeVisible();
    await expect(
      page.getByRole('status').filter({ hasText: 'Waiting for' }),
    ).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath('expired.png') });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('YouTube sign-in failure stops the spinner and allows retry', async ({
  page,
}, testInfo) => {
  const fixture = await providerFixture(page, 'YouTube');
  fixture.state.configured = true;
  fixture.state.pending = true;
  fixture.state.authPhase = 'failed';
  fixture.state.authError = 'Google authorization was declined';
  await page.goto('/');
  await expect(
    page
      .getByRole('alert')
      .filter({ hasText: 'Google authorization was declined' }),
  ).toBeVisible();
  await expect(
    page.getByRole('status').filter({ hasText: 'Waiting for' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Connect YouTube', exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('declined.png') });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('YouTube public video network saves with the other preferences', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'YouTube');
  await page.goto('/');
  await expect(
    page.getByText('Public videos currently use IPv4.', { exact: false }),
  ).toBeVisible();
  await page.getByLabel('Public video network').selectOption('ipv6');
  await expect
    .poll(() => fixture.state.writes.at(-1))
    .toMatchObject({
      path: '/admin/online',
      body: {
        youtube_downloads: false,
        youtube_daily_quota: 10000,
        youtube_network: 'ipv6',
      },
    });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const platform of ['YouTube', 'Twitch', 'Kick']) {
  test(`${platform} integration can be disabled and restored server-wide`, async ({
    page,
  }, testInfo) => {
    const fixture = await providerFixture(page, 'Settings');
    await page.goto('/');
    await page
      .getByRole('link', { name: 'Provider applications', exact: true })
      .click();
    const toggle = page.getByRole('switch', {
      name: `Enable ${platform} integration`,
    });
    await expect(toggle).toBeChecked();
    await toggle.uncheck();
    await expect(
      page
        .getByRole('navigation', { name: 'Main navigation' })
        .getByRole('link', { name: platform, exact: true }),
    ).toHaveCount(0);
    await expect(toggle).not.toBeChecked();
    await page.reload();
    await page
      .getByRole('link', { name: 'Provider applications', exact: true })
      .click();
    await expect(toggle).not.toBeChecked();
    await page
      .getByRole('link', { name: 'Online accounts', exact: true })
      .click();
    await expect(
      page.getByRole('button', { name: `Connect ${platform}`, exact: true }),
    ).toHaveCount(0);
    if (platform === 'Kick')
      await expect(
        page.getByRole('button', { name: 'Manage channels', exact: true }),
      ).toHaveCount(0);
    await page.goto(`/?section=${platform}`);
    await expect(page.locator('.page-header h1')).toHaveText('Home');
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await page
      .getByRole('link', { name: 'Provider applications', exact: true })
      .click();
    await expect(toggle).not.toBeChecked();
    await page.screenshot({
      path: testInfo.outputPath(`${platform.toLowerCase()}-disabled.png`),
      fullPage: true,
    });
    await toggle.check();
    await expect(
      page
        .getByRole('navigation', { name: 'Main navigation' })
        .getByRole('link', { name: platform, exact: true }),
    ).toBeVisible();
    await expect(toggle).toBeChecked();
    await expect(
      page.getByLabel(
        platform === 'YouTube' ? 'Google client ID' : `${platform} client ID`,
      ),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('integration switches recover from failed saves and fit mobile settings', async ({
  page,
}, testInfo) => {
  const fixture = await providerFixture(page, 'Settings');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await page
    .getByRole('link', { name: 'Provider applications', exact: true })
    .click();
  fixture.state.failToggle = true;
  const toggle = page.getByRole('switch', {
    name: 'Enable YouTube integration',
  });
  // A failed save restores the switch, so uncheck() could observe no change.
  await toggle.click();
  await expect(
    page.getByRole('alert').filter({ hasText: 'Settings unavailable' }),
  ).toBeVisible();
  await expect(toggle).toBeChecked();
  fixture.state.failToggle = false;
  await toggle.uncheck();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByLabel('Google client ID')).toHaveCount(0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: testInfo.outputPath('providers-mobile.png'),
    fullPage: true,
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Twitch setup leads directly to device authorization', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'Twitch');
  await page.goto('/');
  await page.getByLabel('Twitch client ID').fill('fixture-twitch');
  await page.getByRole('button', { name: 'Save Twitch settings' }).click();
  await page
    .getByRole('button', { name: 'Connect Twitch', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Enter this code in Twitch' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Copy Twitch device code' }),
  ).toHaveText('TEST-CODE');
  expect(fixture.state.writes.map((write) => write.path)).toEqual([
    '/admin/online/twitch',
    '/online/twitch/connect',
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('regular users get guidance without requests to administrator settings', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'YouTube', false);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await expect(
    page.getByText(/Ask your Thelxinoe administrator/),
  ).toBeVisible();
  await expect(page.getByLabel('Google client ID')).toHaveCount(0);
  fixture.state.configured = true;
  fixture.state.reconnect = true;
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Connect YouTube', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('alert').filter({ hasText: 'Reconnect your account' }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(fixture.state.adminReads).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('missing public URL keeps YouTube setup actionable at mobile width', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'YouTube');
  fixture.state.configured = true;
  fixture.state.linkingAvailable = false;
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await expect(
    page.getByRole('status').filter({ hasText: 'THELXINOE_PUBLIC_URL' }),
  ).toBeVisible();
  await expect(page.getByLabel('Application home page')).toHaveCount(0);
  await expect(
    page.getByRole('link', { name: 'Open privacy policy' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Connect YouTube', exact: true }),
  ).toHaveCount(0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: '.local/provider-setup/youtube-mobile.png',
    fullPage: true,
  });
  await page.getByLabel('Google client secret').scrollIntoViewIfNeeded();
  await page.screenshot({
    path: '.local/provider-setup/youtube-mobile-form.png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Google branding pages are public and readable without JavaScript', async ({
  browser,
}, testInfo) => {
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    javaScriptEnabled: false,
    viewport: { width: 390, height: 844 },
  });
  try {
    const page = await context.newPage();
    const apiRequests: string[] = [];
    page.on('request', (request) => {
      if (new URL(request.url()).pathname.startsWith('/api/'))
        apiRequests.push(request.url());
    });
    const response = await page.goto('/about/index.html');
    expect(response?.status()).toBe(200);
    await expect(
      page.getByRole('heading', { name: 'Thelxinoe', exact: true }),
    ).toBeVisible();
    await expect(page.locator('body')).toHaveCSS('overflow-y', 'auto');
    await page
      .getByRole('link', { name: 'Privacy Policy', exact: true })
      .first()
      .click();
    await expect(page).toHaveURL(/\/about\/privacy\.html$/);
    await expect(
      page.getByRole('heading', { name: 'Privacy Policy', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText('https://www.googleapis.com/auth/youtube.readonly', {
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      page.getByRole('link', { name: 'Google Account connections' }),
    ).toHaveAttribute('href', 'https://myaccount.google.com/connections');
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await testInfo.attach('public-privacy-mobile', {
      body: await page.screenshot({ fullPage: true }),
      contentType: 'image/png',
    });
    await page.getByRole('link', { name: 'Thelxinoe home page' }).click();
    await expect(page).toHaveURL(/\/about\/index\.html$/);
    await page.setViewportSize({ width: 1440, height: 1000 });
    await testInfo.attach('public-home-desktop', {
      body: await page.screenshot({ fullPage: true }),
      contentType: 'image/png',
    });
    expect(apiRequests).toEqual([]);
  } finally {
    await context.close();
  }
});

test('Kick toolbar tracks channels and links administrators to metadata settings', async ({
  page,
}, testInfo) => {
  const fixture = await providerFixture(page, 'Kick');
  await page.goto('/');
  const input = page.getByLabel('Kick channel', { exact: true });
  const setup = page.getByRole('link', {
    name: 'Show live status, titles and thumbnails',
    exact: true,
  });
  const refresh = page.getByRole('button', { name: 'Refresh metadata' });
  await expect(input).toBeVisible();
  await expect(setup).toHaveAttribute('href', '#admin/providers');
  await expect(
    page.getByRole('button', { name: 'Manage channels' }),
  ).toHaveCount(0);
  await expect(page.getByText(/\d+ tracked|metadata is optional/i)).toHaveCount(
    0,
  );
  await expect(refresh).toHaveCount(0);
  await expect(page.getByLabel('Kick client ID')).toHaveCount(0);
  await input.fill('https://kick.com/fixture');
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'fixture', exact: true }),
  ).toBeVisible();
  await expect(input).toHaveValue('');
  await expect(setup).toBeVisible();
  await expect(refresh).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Resume tracking' }),
  ).toHaveCount(0);
  expect(fixture.state.writes[0]).toMatchObject({
    path: '/online/kick/channels',
    body: { channel: 'https://kick.com/fixture' },
  });
  const inputBounds = await input.boundingBox();
  const setupBounds = await setup.boundingBox();
  expect(Math.abs(inputBounds!.y - setupBounds!.y)).toBeLessThan(12);
  await testInfo.attach('kick-unconfigured-admin', {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  await setup.click();
  await expect(page).toHaveURL(/#admin\/providers$/);
  await page.getByLabel('Kick client ID').fill('fixture-kick');
  await page.getByLabel('Kick client secret').fill('fixture-secret');
  await page.getByRole('button', { name: 'Save Kick settings' }).click();
  await expect.poll(() => fixture.state.configured).toBe(true);
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('link', { name: 'Kick', exact: true })
    .click();
  await expect(setup).toHaveCount(0);
  await expect(refresh).toBeEnabled();
  const refreshBounds = await refresh.boundingBox();
  expect(refreshBounds!.x).toBeGreaterThan(inputBounds!.x + inputBounds!.width);
  expect(Math.abs(inputBounds!.y - refreshBounds!.y)).toBeLessThan(2);
  await refresh.click();
  await expect
    .poll(() => fixture.state.writes.at(-1)?.path)
    .toBe('/online/kick/sync');
  fixture.state.kickConnected = false;
  await page.reload();
  await page.getByRole('button', { name: 'Resume tracking' }).click();
  await expect(
    page.getByText('Tracking is paused.', { exact: false }),
  ).toHaveCount(0);
  expect(fixture.state.writes.map((write) => write.path)).toEqual([
    '/online/kick/channels',
    '/admin/online/kick',
    '/online/kick/sync',
    '/online/kick/connect',
  ]);
  const card = page.locator('[data-channel-slug="fixture"]');
  await card.hover();
  await page
    .getByRole('button', { name: 'Stop tracking fixture', exact: true })
    .click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(card).toBeVisible();
  await card.hover();
  await page
    .getByRole('button', { name: 'Stop tracking fixture', exact: true })
    .click();
  await dialog
    .getByRole('button', { name: 'Stop tracking', exact: true })
    .click();
  await expect(card).toHaveCount(0);
  await expect(input).toBeVisible();
  await expect(refresh).toBeDisabled();
  await testInfo.attach('kick-configured-empty', {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('Kick toolbar prompts regular users to ask an administrator and fits mobile', async ({
  page,
}, testInfo) => {
  const fixture = await providerFixture(page, 'Kick', false);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/');
  await expect(
    page.getByText(
      'Ask your administrator to set up Kick to show live status, titles and thumbnails.',
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole('link', { name: 'Show live status, titles and thumbnails' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Refresh metadata' }),
  ).toHaveCount(0);
  await page.getByLabel('Kick channel', { exact: true }).fill('fixture');
  await page.getByLabel('Kick channel', { exact: true }).press('Enter');
  await expect(
    page.getByRole('heading', { name: 'fixture', exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await testInfo.attach('kick-unconfigured-user-mobile', {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  fixture.state.configured = true;
  await page.reload();
  await expect(
    page.getByText(/Ask your administrator to set up Kick/),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Refresh metadata' }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await testInfo.attach('kick-configured-user-mobile', {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  expect(fixture.state.adminReads).toEqual([]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
