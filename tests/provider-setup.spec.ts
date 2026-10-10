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
    if (path === '/online/twitch' || path === '/online/youtube')
      return json({
        configured: state.configured,
        linking_available: state.linkingAvailable,
        account: {
          status: state.reconnect ? 'reconnect_required' : 'disconnected',
          display_name: '',
        },
        pending:
          state.pending && path.endsWith('twitch')
            ? {
                user_code: 'TEST-CODE',
                verification_uri: 'https://www.twitch.tv/activate',
                expires_at: 1999999999,
              }
            : null,
      });
    if (path === '/online/youtube/watchlists') return json([]);
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
  await toggle.uncheck();
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

test('Kick tracking works without credentials and optional metadata saves inline', async ({
  page,
}) => {
  const fixture = await providerFixture(page, 'Kick');
  await page.goto('/');
  const guide = page.getByRole('region', { name: 'kick connection guide' });
  await guide.getByRole('button', { name: 'Manage channels' }).click();
  await page.getByLabel('Kick channel', { exact: true }).fill('fixture');
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(page.getByText('1 tracked', { exact: true })).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Resume tracking' }),
  ).toHaveCount(0);
  expect(fixture.state.writes[0]).toMatchObject({
    path: '/online/kick/channels',
    body: { channel: 'fixture' },
  });
  await page
    .getByText('Optional: show live status, titles and thumbnails')
    .click();
  await page.screenshot({ path: '.local/provider-setup/kick-desktop.png' });
  await page.getByLabel('Kick client ID').fill('fixture-kick');
  await page.getByLabel('Kick client secret').fill('fixture-secret');
  await page.getByRole('button', { name: 'Save Kick settings' }).click();
  await expect(guide).toHaveCount(0);
  fixture.state.kickConnected = false;
  await page.reload();
  await page.getByRole('button', { name: 'Resume tracking' }).click();
  await expect(
    page.getByText('Tracking is paused.', { exact: false }),
  ).toHaveCount(0);
  expect(fixture.state.writes.map((write) => write.path)).toEqual([
    '/online/kick/channels',
    '/admin/online/kick',
    '/online/kick/connect',
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
