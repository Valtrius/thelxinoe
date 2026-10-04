import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

async function desktopFixture(page: Page, signedIn = false) {
  const fixture = await installUiFixture(page, { signedIn });
  await page.addInitScript(() => {
    const bridge = window as unknown as {
      isTauri: boolean;
      startupReveals: { text: string; iconsReady: boolean; theme: string }[];
      __TAURI_INTERNALS__: unknown;
      __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
    };
    bridge.isTauri = true;
    bridge.startupReveals = [];
    bridge.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    let callback = 0;
    bridge.__TAURI_INTERNALS__ = {
      metadata: {
        currentWindow: { label: 'main' },
        currentWebview: { label: 'main' },
      },
      transformCallback: () => ++callback,
      unregisterCallback() {},
      async invoke(command: string, args: Record<string, unknown> = {}) {
        if (command === 'server_url') return location.origin;
        if (command === 'backend_request') {
          const response = await fetch(`/api/v1${args.path}`, {
            method: String(args.method),
            headers: { 'Content-Type': 'application/json' },
            ...(args.body ? { body: JSON.stringify(args.body) } : {}),
          });
          return { status: response.status, body: await response.json() };
        }
        if (command === 'finish_startup') {
          bridge.startupReveals.push({
            text: document.body.innerText,
            iconsReady: Array.from(
              document.querySelectorAll<HTMLImageElement>(
                'img[src="/icon.svg"]',
              ),
            ).every((image) => image.complete && image.naturalWidth > 0),
            theme: document.documentElement.dataset.theme ?? '',
          });
          return;
        }
        if (command === 'mpv_state') return { status: 'stopped', media_id: '' };
        if (command === 'tools_get') return { tools: [], mpv: {} };
        if (command === 'plugin:app|version') return '0.1.0';
        if (command === 'desktop_update_status') return null;
        if (command === 'plugin:event|listen') return ++callback;
        return false;
      },
    };
    localStorage.setItem(
      `thelxinoe:${location.origin}:layout-fixture:navigation`,
      JSON.stringify({ section: 'Settings', settingsSection: 'account' }),
    );
  });
  return fixture;
}

function reveals(page: Page) {
  return page.evaluate(
    () =>
      (
        window as unknown as {
          startupReveals: {
            text: string;
            iconsReady: boolean;
            theme: string;
          }[];
        }
      ).startupReveals,
  );
}

test('window controls use the standard arrow cursor', async ({
  page,
}, testInfo) => {
  const fixture = await desktopFixture(page, true);
  await page.goto('/');
  for (const name of ['Minimize window', 'Maximize window', 'Close window']) {
    const control = page.getByRole('button', { name, exact: true });
    await control.hover();
    await expect(control).toHaveCSS('cursor', 'default');
  }
  await testInfo.attach('window-controls', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('desktop waits for its connection screen and icon before revealing the window', async ({
  page,
}, testInfo) => {
  const fixture = await desktopFixture(page);
  let releaseHealth!: () => void;
  const healthGate = new Promise<void>((resolve) => (releaseHealth = resolve));
  await page.route('**/api/v1/health', async (route) => {
    await healthGate;
    await route.fulfill({ json: { api_version: 1 } });
  });
  let releaseIcon!: () => void;
  const iconGate = new Promise<void>((resolve) => (releaseIcon = resolve));
  await page.route('**/icon.svg', async (route) => {
    await iconGate;
    await route.continue();
  });
  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await expect(page.getByText('Connecting to your library…')).toBeVisible();
  expect(await reveals(page)).toEqual([]);
  releaseHealth();
  await expect(
    page.getByRole('heading', { name: 'Welcome back' }),
  ).toBeVisible();
  expect(await reveals(page)).toEqual([]);
  releaseIcon();
  await expect.poll(() => reveals(page)).toHaveLength(1);
  const [reveal] = await reveals(page);
  expect(reveal.text).toContain('Server address');
  expect(reveal.text).toContain('Welcome back');
  expect(reveal.text).not.toContain('Connecting to your library…');
  expect(reveal.iconsReady).toBe(true);
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('connection-ready', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('desktop restores the signed-in appearance and navigation before revealing the window', async ({
  page,
}, testInfo) => {
  const fixture = await desktopFixture(page, true);
  let releasePreferences!: () => void;
  const gate = new Promise<void>((resolve) => (releasePreferences = resolve));
  await page.route('**/api/v1/me/preferences', async (route) => {
    await gate;
    await route.fulfill({ json: { timezone: 'UTC', time_format: '24h' } });
  });
  await page.route('**/api/v1/me/appearance', (route) =>
    route.fulfill({ json: { theme: 'dark', sidebar_collapsed: true } }),
  );
  await page.goto('/');
  await expect(page.getByText('Connecting to your library…')).toBeVisible();
  expect(await reveals(page)).toEqual([]);
  releasePreferences();
  await expect.poll(() => reveals(page)).toHaveLength(1);
  const [reveal] = await reveals(page);
  expect(reveal.theme).toBe('dark');
  expect(reveal.text).toContain('Your display preferences');
  await expect(
    page.getByRole('button', { name: 'Sign out', exact: true }),
  ).toBeVisible();
  expect(reveal.text).not.toContain('Connecting to your library…');
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('signed-in-ready', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

for (const scenario of ['offline', 'incompatible', 'setup'] as const) {
  test(`desktop reveals a usable ${scenario} screen after startup`, async ({
    page,
  }, testInfo) => {
    const fixture = await desktopFixture(page);
    if (scenario === 'offline')
      await page.route('**/api/v1/health', (route) =>
        route.fulfill({
          status: 503,
          json: {
            error: { message: 'Cannot connect to the configured server' },
          },
        }),
      );
    if (scenario === 'incompatible')
      await page.route('**/api/v1/health', (route) =>
        route.fulfill({ json: { api_version: 99 } }),
      );
    if (scenario === 'setup')
      await page.route('**/api/v1/setup', (route) =>
        route.fulfill({ json: { setup_required: true } }),
      );
    await page.goto('/');
    await expect.poll(() => reveals(page)).toHaveLength(1);
    const [reveal] = await reveals(page);
    expect(reveal.text).not.toContain('Connecting to your library…');
    if (scenario === 'offline') {
      expect(reveal.text).toContain('Cannot connect to the configured server');
      await expect(page.getByLabel('Server address')).toBeEditable();
    } else if (scenario === 'incompatible') {
      expect(reveal.text).toContain('Update required');
      await expect(
        page.getByRole('button', { name: 'Connect to server' }),
      ).toBeEnabled();
    } else {
      expect(reveal.text).toContain('Welcome to Thelxinoe');
      await expect(
        page.getByLabel('Confirm password', { exact: true }),
      ).toBeEditable();
    }
    expect(fixture.errors).toEqual([]);
    await testInfo.attach(`${scenario}-ready`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

for (const colorScheme of ['dark', 'light'] as const) {
  test(`splashscreen uses the YouTwitch template in ${colorScheme} mode`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 420, height: 240 });
    await page.emulateMedia({ colorScheme });
    await page.goto('/splashscreen.html');
    await expect(
      page.getByRole('heading', { name: 'Preparing your library' }),
    ).toBeVisible();
    await expect(page.getByLabel('Loading')).toBeVisible();
    const icon = page.locator('img');
    await expect
      .poll(() => icon.evaluate((image) => image.naturalWidth))
      .toBeGreaterThan(0);
    expect(
      await page.evaluate(() => document.documentElement.scrollHeight),
    ).toBe(240);
    expect(
      await page.evaluate(
        () => getComputedStyle(document.body).backgroundColor,
      ),
    ).toBe(colorScheme === 'dark' ? 'rgb(5, 7, 11)' : 'rgb(237, 241, 243)');
    await testInfo.attach(`splash-${colorScheme}`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(page.locator('i')).toHaveCSS('animation-name', 'none');
  });
}
