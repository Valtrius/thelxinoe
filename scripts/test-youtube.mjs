// Isolated fixture deployment only. Never run against a server with real users.
import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const origin = 'https://localhost:22443';
const browser = await chromium.launch();
const owner = await browser.newContext({ ignoreHTTPSErrors: true });
const guest = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await owner.newPage();
const headers = { 'X-Thelxinoe-Client': '1' };
async function api(context, path, method = 'GET', data) {
  const response = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    headers,
    data,
  });
  if (!response.ok()) throw new Error(`${path}: HTTP ${response.status()}`);
  return response.json();
}
try {
  const setup = await api(owner, '/setup');
  if (setup.setup_required) {
    const code = execFileSync(
      'docker',
      [
        'compose',
        '-p',
        'thelxinoe-online',
        '-f',
        'compose.test.yaml',
        'exec',
        '-T',
        'server',
        'cat',
        '/var/lib/thelxinoe/secrets/setup-token',
      ],
      { encoding: 'utf8' },
    ).trim();
    await api(owner, '/setup', 'POST', {
      username: 'admin',
      password: 'test-only long passphrase',
      setup_token: code,
    });
  } else
    await api(owner, '/auth/login', 'POST', {
      username: 'admin',
      password: 'test-only long passphrase',
    });
  const existing = await api(owner, '/online/youtube');
  if (existing.account.status !== 'disconnected')
    throw new Error(
      'The fixture account is connected; refusing to replace its credentials or delete its data.',
    );
  await api(owner, '/admin/online', 'PUT', {
    google: {
      client_id: 'fixture.apps.googleusercontent.com',
      client_secret: 'fixture-application-secret',
    },
    youtube_downloads: true,
    youtube_daily_quota: 10000,
  });
  await api(owner, '/online/youtube/data', 'DELETE');
  const users = (await api(owner, '/users')).items;
  if (!users.some((u) => u.username === 'online-user'))
    await api(owner, '/users', 'POST', {
      username: 'online-user',
      password: 'test-only long passphrase',
      role: 'user',
    });
  await api(guest, '/auth/login', 'POST', {
    username: 'online-user',
    password: 'test-only long passphrase',
  });
  const denied = await guest.request.put(`${origin}/api/v1/admin/online`, {
    headers,
    data: { youtube_downloads: false, youtube_daily_quota: 1 },
  });
  expect(denied.status()).toBe(403);
  await page.goto(`${origin}/?section=YouTube`);
  await expect(page.locator('.provider-account').first()).toBeVisible();
  const releases = [];
  await page.route('**/api/v1/online/youtube/watchlist', async (route) => {
    await new Promise((resolve) => releases.push(resolve));
    await route.continue();
  });
  const input = page.getByRole('textbox', { name: 'YouTube video URL' });
  await input.fill('https://youtu.be/abcdefghijk');
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await expect(
    page.getByText('https://youtu.be/abcdefghijk', { exact: true }),
  ).toBeVisible();
  await input.fill('https://youtu.be/lmnopqrstuv');
  await input.press('Enter');
  await expect(input).toHaveValue('');
  await expect.poll(() => releases.length).toBe(2);
  releases[1]();
  await expect(
    page.getByRole('heading', { name: 'lmnopqrstuv', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText('https://youtu.be/abcdefghijk', { exact: true }),
  ).toBeVisible();
  releases[0]();
  await page.getByRole('button', { name: 'Watchlist', exact: true }).click();
  await expect(
    page
      .locator('.youtube-feed')
      .getByRole('heading', { name: 'abcdefghijk', exact: true }),
  ).toBeVisible();
  await page.unroute('**/api/v1/online/youtube/watchlist');
  expect((await api(guest, '/online/youtube/feed?watchlist=true')).total).toBe(
    0,
  );
  await page
    .getByRole('button', { name: 'Pin abcdefghijk', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Pin abcdefghijk', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await page
    .getByRole('button', { name: 'Watched abcdefghijk', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Watched abcdefghijk', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  let authorization;
  await page.route('https://accounts.google.com/**', async (route) => {
    authorization = new URL(route.request().url());
    expect(authorization.searchParams.get('redirect_uri')).toBe(
      `${origin}/api/v1/online/youtube/callback`,
    );
    expect(authorization.searchParams.get('code_challenge_method')).toBe(
      'S256',
    );
    const callback = new URL(authorization.searchParams.get('redirect_uri'));
    callback.searchParams.set('state', authorization.searchParams.get('state'));
    callback.searchParams.set('error', 'access_denied');
    await route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><title>Authorization fixture</title><a href="${callback.href.replaceAll('&', '&amp;')}">Decline fixture authorization</a>`,
    });
  });
  const callbackSeen = page.waitForRequest((r) =>
    r.url().startsWith(`${origin}/api/v1/online/youtube/callback?`),
  );
  await page
    .getByRole('button', { name: 'Connect YouTube', exact: true })
    .click();
  await expect(
    page.getByRole('link', { name: 'Decline fixture authorization' }),
  ).toBeVisible();
  const binding = (await owner.cookies()).find(
    (c) => c.name === 'thelxinoe_youtube_oauth',
  );
  expect(binding.httpOnly && binding.secure).toBe(true);
  expect(binding.sameSite).toBe('Lax');
  expect(binding.path).toBe('/api/v1/online/youtube/callback');
  await page
    .getByRole('link', { name: 'Decline fixture authorization' })
    .click();
  const callbackHeaders = await (await callbackSeen).allHeaders();
  expect(callbackHeaders.cookie).toContain('thelxinoe_youtube_oauth=');
  expect(callbackHeaders.cookie).not.toContain('thelxinoe_session=');
  await expect(
    page.getByText(/Account linking did not complete\./),
  ).toBeVisible();
  await expect(page.locator('.provider-account').first()).toBeVisible();
  expect(
    (await owner.cookies()).some((c) => c.name === 'thelxinoe_youtube_oauth'),
  ).toBe(false);
  const replay = await owner.request.get(
    `${origin}/api/v1/online/youtube/callback?state=${authorization.searchParams.get('state')}&code=fixture-replay`,
    { maxRedirects: 0 },
  );
  expect(replay.status()).toBe(303);
  expect(replay.headers().location).toBe('/?youtube_link=failed');
  await api(owner, '/online/youtube', 'DELETE');
  expect((await api(owner, '/online/youtube/feed?watchlist=true')).total).toBe(
    2,
  );
  await page.getByRole('button', { name: 'Watchlist', exact: true }).click();
  await expect(
    page
      .locator('.youtube-feed')
      .getByRole('heading', { name: 'abcdefghijk', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: '.local/youtube-watchlist.png',
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Online accounts', exact: true })
    .click();
  await expect(
    page.getByRole('textbox', { name: 'Authorized redirect URI' }),
  ).toHaveValue(`${origin}/api/v1/online/youtube/callback`);
  await expect(
    page.getByRole('textbox', { name: 'Google client secret' }),
  ).toHaveValue('');
  await page.screenshot({
    path: '.local/youtube-settings.png',
    fullPage: true,
  });
  const result = {
    https_proxy: true,
    browser_cookie_binding: true,
    strict_session_survives_callback: true,
    pkce: true,
    single_use: true,
    concurrent_optimistic_adds: true,
    private_watchlists: true,
    private_pins_and_watched: true,
    admin_only_configuration: true,
    disconnect_retains_data: true,
    real_google_consent: false,
  };
  writeFileSync('.local/youtube-result.json', JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally {
  await browser.close();
}
