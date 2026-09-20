import { test, expect } from '@playwright/test';

// Run only against the explicitly selected disposable UI fixture (see TESTING.md).
test.beforeEach(async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('navigation', { name: 'Main navigation' }),
  ).toBeVisible();
});

test('appearance persists, native menus stay hidden on web, and cards animate during zoom and sidebar resize', async ({
  page,
}) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.getByRole('button', { name: 'Dark theme', exact: true }).click();
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json()).theme,
    )
    .toBe('dark');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.getByRole('button', { name: /MPV/i })).toHaveCount(0);
  await expect(page.getByText(/yt-dlp/i)).toHaveCount(0);
  await page.getByRole('button', { name: 'Light theme', exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        getComputedStyle(document.documentElement)
          .getPropertyValue('--background')
          .trim(),
      ),
    )
    .toBe('#edf1f3');
  await page.getByRole('button', { name: 'Dark theme', exact: true }).click();
  await page
    .getByRole('button', { name: 'Movies', exact: true })
    .first()
    .click();
  const cards = page.locator('.library-tile');
  await expect(cards.first()).toBeVisible();
  // Normalize density through the supported account preference, then reload.
  await page.request.patch('/api/v1/me/appearance', {
    headers: { 'X-Thelxinoe-Client': '1' },
    data: { card_columns: 6, sidebar_collapsed: false },
  });
  await page.reload();
  await page
    .getByRole('button', { name: 'Movies', exact: true })
    .first()
    .click();
  await expect(cards.first()).toBeVisible();
  await page.waitForTimeout(250);
  const initial = await cards.first().boundingBox();
  const samples = await page.evaluate(async () => {
    const rect = () => {
      const b = document
        .querySelector('.library-tile')!
        .getBoundingClientRect();
      return { x: b.x, width: b.width };
    };
    const samples = [rect()];
    (
      document.querySelector('[aria-label="Collapse sidebar"]') as HTMLElement
    ).click();
    const start = performance.now();
    while (performance.now() - start < 350) {
      await new Promise(requestAnimationFrame);
      samples.push(rect());
    }
    return samples;
  });
  const final = samples.at(-1)!;
  expect(final.x).toBeLessThan(initial!.x);
  expect(final.width).toBeGreaterThan(initial!.width);
  expect(samples.some((p) => p.x > final.x + 1 && p.x < initial!.x - 1)).toBe(
    true,
  );
  expect(
    samples.every(
      (p) => p.width > 0 && p.x >= final.x - 2 && p.x <= initial!.x + 2,
    ),
  ).toBe(true);
  await cards.first().hover();
  await expect(cards.first().locator('.card-actions')).toHaveCSS(
    'opacity',
    '1',
  );
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -100);
  await page.keyboard.up('Control');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .card_columns,
    )
    .toBe(5);
  await expect
    .poll(async () => (await cards.first().boundingBox())!.width)
    .toBeGreaterThan(final.width);
  await page.screenshot({ path: '.local/ui-validation/movies-dark.png' });
  await page.getByRole('button', { name: 'Light theme', exact: true }).click();
  await page.waitForTimeout(250);
  await page.screenshot({ path: '.local/ui-validation/movies-light.png' });
  await page.setViewportSize({ width: 480, height: 850 });
  await page.waitForTimeout(250);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: '.local/ui-validation/movies-mobile.png' });
  expect(errors).toEqual([]);
});

test('YouTube keeps optimistic additions visible and clears input while requests are pending', async ({
  page,
  context,
}) => {
  await page
    .getByRole('button', { name: 'YouTube', exact: true })
    .first()
    .click();
  const input = page.getByRole('textbox', { name: 'YouTube video URL' });
  await expect(input).toBeVisible();
  const cdp = await context.newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.emulateNetworkConditions', {
    offline: false,
    latency: 800,
    downloadThroughput: -1,
    uploadThroughput: -1,
  });
  await input.fill('https://youtu.be/UItest00001');
  await page
    .getByRole('button', { name: 'Add to watchlist', exact: true })
    .click();
  await expect(input).toHaveValue('');
  await expect(
    page.locator('.watchlist-pending').filter({ hasText: 'UItest00001' }),
  ).toBeVisible();
  await input.fill('https://youtu.be/UItest00002');
  await page
    .getByRole('button', { name: 'Add to watchlist', exact: true })
    .click();
  await expect(input).toHaveValue('');
  await expect(
    page.locator('.watchlist-pending').filter({ hasText: 'UItest00002' }),
  ).toBeVisible();
  await expect(page.locator('.watchlist-pending')).toHaveCount(0, {
    timeout: 15000,
  });
  await cdp.send('Network.emulateNetworkConditions', {
    offline: false,
    latency: 0,
    downloadThroughput: -1,
    uploadThroughput: -1,
  });
  await page.screenshot({ path: '.local/ui-validation/youtube-populated.png' });
  await page.getByRole('button', { name: 'Hide watchlist' }).click();
  await expect(
    page.getByRole('button', { name: 'Show watchlist' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Show watchlist' }).click();
  await expect(input).toBeVisible();
});
