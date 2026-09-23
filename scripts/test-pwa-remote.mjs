import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync, mkdirSync, existsSync } from 'node:fs';
const origin =
  process.env.THELXINOE_RELEASE_ORIGIN || 'https://localhost:28443';
const project = process.env.THELXINOE_RELEASE_PROJECT || 'thelxinoe-release-v6';
const root = process.env.THELXINOE_RELEASE_STATE || '.local/releases-v6';
const browser = await chromium.launch({
  args: ['--ignore-certificate-errors'],
});
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 390, height: 844 },
  isMobile: true,
  hasTouch: true,
});
const page = await context.newPage();
async function api(path, method = 'GET', data) {
  const response = await context.request
    .fetch(origin + '/api/v1' + path, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
    })
    .catch(() => {
      throw Error('PWA fixture transport unavailable');
    });
  if (!response.ok())
    throw Error(
      `${path}: ${response.status()} ${(await response.text()).slice(0, 200)}`,
    );
  return response.json();
}
const sessions = [];
try {
  await page.goto(origin);
  await page.getByLabel('Username', { exact: true }).waitFor();
  const setup = await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .isVisible();
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  if (setup)
    await page
      .getByLabel('Confirm password', { exact: true })
      .fill('test-only long passphrase');
  await page
    .getByRole('button', {
      name: setup ? 'Create your server' : 'Sign in',
      exact: true,
    })
    .click();
  await expect(page.getByRole('heading', { name: 'Discover' })).toBeVisible();
  expect(
    (await context.cookies()).find((c) => c.name === 'thelxinoe_session'),
  ).toMatchObject({ secure: true, httpOnly: true, sameSite: 'Strict' });
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.reload();
  await expect
    .poll(() => page.evaluate(() => !!navigator.serviceWorker.controller))
    .toBe(true);
  const manifest = await (
    await context.request.get(origin + '/manifest.webmanifest')
  ).json();
  expect(manifest.display).toBe('standalone');
  expect(
    manifest.icons.filter((i) => i.type === 'image/png').map((i) => i.sizes),
  ).toEqual(['192x192', '512x512']);
  const cdp = await context.newCDPSession(page);
  const install = await cdp.send('Page.getInstallabilityErrors');
  expect(install.installabilityErrors).toEqual([]);
  for (const section of [
    'Home',
    'Movies',
    'Shows',
    'Music',
    'YouTube',
    'Twitch',
    'Kick',
    'Settings',
  ]) {
    if (section !== 'Home')
      await page
        .getByRole('button', { name: section, exact: true })
        .first()
        .click();
    await expect
      .poll(() =>
        page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth + 1,
        ),
      )
      .toBe(true);
  }
  await page.screenshot({
    path: '.local/pwa-mobile-settings.png',
    fullPage: true,
  });
  const cached = await page.evaluate(async () => {
    const all = [];
    for (const key of await caches.keys())
      for (const request of await (await caches.open(key)).keys())
        all.push(new URL(request.url).pathname);
    return all;
  });
  expect(cached).toEqual(['/offline.html']);
  await context.setOffline(true);
  await page.goto(origin);
  await expect(
    page.getByRole('heading', { name: 'Connect to your server' }),
  ).toBeVisible();
  await context.setOffline(false);
  await page.getByRole('link', { name: 'Reconnect' }).click();
  await expect(page.getByRole('heading', { name: 'Discover' })).toBeVisible();
  expect(
    (
      await context.request.post(origin + '/api/v1/admin/jobs', {
        headers: {
          Origin: 'https://untrusted.example',
          'X-Thelxinoe-Client': '1',
        },
        data: { key: 'forbidden' },
      })
    ).status(),
  ).toBe(403);
  expect(
    (
      await context.request.get(origin + '/api/v1/auth/me', {
        headers: { 'X-Thelxinoe-API': '999' },
      })
    ).status(),
  ).toBe(426);
  mkdirSync(`${root}/data/remote`, { recursive: true });
  const file = `${root}/data/remote/Remote Fixture (2020).mp4`;
  if (!existsSync(file))
    execFileSync(
      'ffmpeg',
      [
        '-hide_banner',
        '-loglevel',
        'error',
        '-f',
        'lavfi',
        '-i',
        'testsrc2=size=640x360:rate=24',
        '-f',
        'lavfi',
        '-i',
        'sine=frequency=440:sample_rate=48000',
        '-t',
        '8',
        '-c:v',
        'libx264',
        '-preset',
        'ultrafast',
        '-c:a',
        'aac',
        '-movflags',
        '+faststart',
        file,
      ],
      { stdio: 'pipe' },
    );
  const roots = (await api('/catalog/roots')).items;
  const library =
    roots.find((r) => r.path === '/media/remote') ||
    (await api('/catalog/roots', 'POST', {
      name: 'Remote fixture',
      kind: 'movies',
      path: '/media/remote',
    }));
  await api(`/catalog/roots/${library.id}/scan`, 'POST');
  await expect
    .poll(async () => (await api('/catalog?kind=movie')).items.length, {
      timeout: 60000,
    })
    .toBeGreaterThan(0);
  const movie = (await api('/catalog?kind=movie')).items.find(
    (i) => i.title === 'Remote Fixture',
  );
  expect(movie).toBeTruthy();
  const options = {
    quality: 'auto',
    audio: null,
    subtitle: null,
    capabilities: {
      containers: ['mp4'],
      video: ['h264'],
      audio: ['aac'],
      hls: true,
    },
  };
  const direct = await api('/playback', 'POST', {
    media_id: movie.id,
    options,
  });
  sessions.push({ id: direct.id });
  expect(direct.mode).toBe('direct');
  const ranged = await context.request.get(new URL(direct.url, origin).href, {
    headers: { Range: 'bytes=0-1023' },
  });
  expect(ranged.status()).toBe(206);
  expect((await ranged.body()).length).toBe(1024);
  const login = await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
    transport: 'device',
    device_name: 'Remote quality fixture',
  });
  let remote;
  try {
    remote = JSON.parse(
      execFileSync(
        'docker',
        [
          'exec',
          '-i',
          `${project}-server-1`,
          'curl',
          '-fsS',
          '--config',
          '-',
          '--json',
          JSON.stringify({ media_id: movie.id, options }),
        ],
        {
          encoding: 'utf8',
          input: `url = "http://server:8484/api/v1/playback"\nheader = "Authorization: Bearer ${login.token}"\nheader = "X-Thelxinoe-Client: 1"\nheader = "X-Forwarded-For: 8.8.8.8"\n`,
          stdio: ['pipe', 'pipe', 'pipe'],
        },
      ),
    );
  } catch {
    throw Error('Remote quality fixture request failed');
  }
  sessions.push({ id: remote.id, token: login.token });
  expect(remote.mode).toBe('transcode');
  let playlist;
  await expect
    .poll(
      async () => {
        const r = await context.request.get(new URL(remote.url, origin).href);
        playlist = await r.text();
        return r.status();
      },
      { timeout: 60000, intervals: [1000] },
    )
    .toBe(200);
  expect(playlist).toContain('#EXTM3U');
  const segment = playlist
    .split(/\r?\n/)
    .find((line) => line && !line.startsWith('#'));
  const chunk = await context.request.get(
    new URL(segment, new URL(remote.url, origin)).href,
  );
  expect(chunk.status()).toBe(200);
  expect((await chunk.body()).length).toBeGreaterThan(100);
  // A server upgrade can change compatibility while an existing page is open.
  await page.route('**/api/v1/catalog?*', (route) =>
    route.fulfill({
      status: 426,
      contentType: 'application/json',
      body: JSON.stringify({
        error: {
          code: 'update_required',
          message: 'Update this client to continue.',
        },
      }),
    }),
  );
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('button', { name: 'Movies', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Update required' }),
  ).toBeVisible();
  await page.screenshot({ path: '.local/pwa-update-required.png' });
  writeFileSync(
    '.local/pwa-remote-result.json',
    JSON.stringify(
      {
        passed: true,
        firstRunWizard: setup,
        mobileNavigation: true,
        installable: true,
        publicOfflineScreen: true,
        secureCookies: true,
        proxyRange: true,
        proxyHls: true,
        remoteAutoBitrate: true,
        activeClientUpdateRequired: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'PWA installation, mobile layout, offline reconnect, HTTPS range/HLS and remote Auto quality passed',
  );
} finally {
  for (const session of sessions)
    await context.request
      .post(`${origin}/api/v1/playback/${session.id}/progress`, {
        headers: {
          'X-Thelxinoe-Client': '1',
          ...(session.token
            ? { Authorization: `Bearer ${session.token}` }
            : {}),
        },
        data: { sequence: 99, position: 0, state: 'stopped' },
      })
      .catch(() => {});
  await browser.close();
}
