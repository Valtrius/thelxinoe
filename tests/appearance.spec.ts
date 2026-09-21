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

test('sidebar footer keeps its rows and animates controls without stretching icons', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1360, height: 900 });
  await page.request.patch('/api/v1/me/appearance', {
    headers: { 'X-Thelxinoe-Client': '1' },
    data: { sidebar_collapsed: false },
  });
  await expect(page.locator('.primary-sidebar')).not.toHaveClass(/collapsed/);
  for (const label of ['Collapse sidebar', 'Expand sidebar']) {
    const samples = await page.evaluate(async (label) => {
      const snapshot = () => {
        const rect = (element: Element) => {
          const r = element.getBoundingClientRect();
          return { x: r.x, y: r.y, width: r.width, height: r.height };
        };
        return {
          buttons: [
            ...document.querySelectorAll('.web-theme-controls button'),
          ].map(rect),
          icons: [...document.querySelectorAll('.web-theme-controls svg')].map(
            rect,
          ),
          avatar: rect(document.querySelector('.sidebar-profile .avatar')!),
          logout: rect(document.querySelector('.sidebar-profile button')!),
          statistics: rect(
            document.querySelector('[aria-label="Statistics"]')!,
          ),
          navigation: rect(document.querySelector('.primary-navigation')!),
        };
      };
      const frames = [snapshot()];
      (
        document.querySelector(`[aria-label="${label}"]`) as HTMLElement
      ).click();
      const start = performance.now();
      while (performance.now() - start < 300) {
        await new Promise(requestAnimationFrame);
        frames.push(snapshot());
      }
      return frames;
    }, label);
    const first = samples[0],
      last = samples.at(-1)!;
    for (const frame of samples) {
      expect(frame.statistics.y).toBeCloseTo(first.statistics.y, 0);
      expect(frame.navigation.height).toBeCloseTo(first.navigation.height, 0);
      for (const button of frame.buttons) {
        expect(button.y).toBeCloseTo(first.buttons[0].y, 0);
        expect(button.height).toBe(32);
      }
      for (const icon of frame.icons) {
        expect(icon.width).toBeCloseTo(14, 0);
        expect(icon.height).toBeCloseTo(14, 0);
      }
    }
    const moves = [
      (s: typeof first) => s.buttons[0].x,
      (s: typeof first) => s.buttons[2].x,
      (s: typeof first) => s.avatar.x,
      (s: typeof first) => s.logout.x,
    ];
    for (const position of moves) {
      const lo = Math.min(position(first), position(last));
      const hi = Math.max(position(first), position(last));
      expect(hi - lo).toBeGreaterThan(5);
      expect(
        samples.some((s) => position(s) > lo + 1 && position(s) < hi - 1),
      ).toBe(true);
      expect(
        samples.every((s) => position(s) >= lo - 1 && position(s) <= hi + 1),
      ).toBe(true);
    }
    expect(last.buttons[0].width).toBe(label === 'Collapse sidebar' ? 20 : 32);
  }
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page
    .getByRole('button', { name: 'Collapse sidebar', exact: true })
    .click();
  await expect(page.locator('.primary-sidebar')).toHaveClass(/collapsed/);
  expect(
    await page
      .locator('.sidebar-bottom')
      .evaluate(
        (e) =>
          e
            .getAnimations({ subtree: true })
            .filter(
              (a) =>
                a instanceof Animation &&
                a.effect instanceof KeyframeEffect &&
                a.effect.getKeyframes().some((k) => 'transform' in k),
            ).length,
      ),
  ).toBe(0);
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

test('YouTube keeps concurrent optimistic additions visible and saves named lists', async ({
  page,
}) => {
  const lists = await (
    await page.request.get('/api/v1/online/youtube/watchlists')
  ).json();
  await page.request.patch('/api/v1/me/appearance', {
    headers: { 'X-Thelxinoe-Client': '1' },
    data: {
      provider_preferences: {
        'youtube-watchlist-sidebar-open': 'false',
        'youtube-selected-watchlist-id': String(
          lists.find((list: { isDefault: boolean }) => list.isDefault).id,
        ),
      },
    },
  });
  await page.route('**/api/v1/online/youtube', (route) =>
    route.fulfill({
      json: {
        configured: false,
        downloads_enabled: false,
        account: { status: 'connected', display_name: 'UI fixture' },
      },
    }),
  );
  await page.goto('/?section=YouTube');
  await page
    .getByRole('button', { name: 'Open watchlists', exact: true })
    .click();
  const input = page.getByRole('searchbox');
  const ids = [
    crypto.randomUUID().replaceAll('-', '').slice(0, 11),
    crypto.randomUUID().replaceAll('-', '').slice(0, 11),
  ];
  const releases: (() => void)[] = [];
  await page.route('**/online/youtube/watchlists/*/items', async (route) => {
    await new Promise<void>((resolve) => releases.push(resolve));
    await route.continue();
  });
  for (const id of ids) {
    await input.fill(`https://youtu.be/${id}`);
    await input.press('Control+Enter');
    await expect(input).toHaveValue('');
    await expect(
      page.locator(`[data-watchlist-video-id="${id}"]`),
    ).toBeVisible();
  }
  await expect.poll(() => releases.length).toBe(2);
  releases.forEach((release) => release());
  await expect
    .poll(async () => {
      const lists = await (
        await page.request.get('/api/v1/online/youtube/watchlists')
      ).json();
      return lists
        .find((list: { isDefault: boolean }) => list.isDefault)
        .items.filter((item: { video: { videoId: string } }) =>
          ids.includes(item.video.videoId),
        ).length;
    })
    .toBe(2);
  await page.unroute('**/online/youtube/watchlists/*/items');
  await page
    .locator('[data-youtube-watchlist-frame]')
    .getByRole('button', { name: /^Watch Later/ })
    .click();
  const listName = `Reading ${ids[0]}`;
  await page.getByPlaceholder('New watchlist').fill(listName);
  await page
    .getByRole('button', { name: 'Create watchlist', exact: true })
    .click();
  await expect(
    page.getByRole('dialog', { name: 'Watchlists', exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .locator('[data-youtube-watchlist-frame]')
      .getByRole('button', { name: new RegExp(`^${listName}`) }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Close watchlists' }).click();
  await expect(
    page.getByRole('button', { name: 'Open watchlists' }),
  ).toBeVisible();
  await page.screenshot({ path: '.local/ui-validation/youtube-populated.png' });
});

test('notifications dismiss outside and Settings panels stay at the left', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const panels = page.locator('.settings-panels');
  await expect(panels).toBeVisible();
  const geometry = await panels.evaluate((element) => {
    const rect = element.getBoundingClientRect(),
      parent = element.parentElement!.getBoundingClientRect();
    return { width: rect.width, left: rect.left - parent.left };
  });
  expect(geometry.width).toBeLessThanOrEqual(880);
  expect(geometry.left).toBe(24);
  const trigger = page.getByRole('button', { name: /^Notifications/ });
  await trigger.click();
  await expect(page.locator('.notices')).toBeVisible();
  await page.getByRole('heading', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.notices')).toHaveCount(0);
  await trigger.click();
  await page.keyboard.press('Escape');
  await expect(page.locator('.notices')).toHaveCount(0);
  await expect(trigger).toBeFocused();
});

test('provider Ctrl+wheel zoom has one owner and sidebar motion keeps cards visible', async ({
  page,
}) => {
  await page.route('**/api/v1/online/twitch', (route) =>
    route.fulfill({
      json: {
        configured: false,
        account: { status: 'connected', display_name: 'UI fixture' },
      },
    }),
  );
  await page.route('**/api/v1/online/twitch/feed', (route) =>
    route.fulfill({
      json: {
        items: Array.from({ length: 12 }, (_, i) => ({
          id: `fixture-${i}`,
          login: `fixture${i}`,
          display_name: `Channel ${i}`,
          title: 'A stream for layout testing',
          category: 'Science & Technology',
          viewers: 100 + i,
          started_at: '2026-09-20T12:00:00Z',
        })),
      },
    }),
  );
  await page.request.patch('/api/v1/me/appearance', {
    headers: { 'X-Thelxinoe-Client': '1' },
    data: { card_columns: 6, sidebar_collapsed: false },
  });
  await page.reload();
  await page.getByRole('button', { name: 'Twitch', exact: true }).click();
  const card = page.locator('[data-stream-id]').first();
  await expect(card).toBeVisible();
  const before = (await card.boundingBox())!;
  await card.hover();
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
    .poll(async () => (await card.boundingBox())!.width)
    .toBeGreaterThan(before.width);
  await page.waitForTimeout(250);
  const samples = await page.evaluate(async () => {
    const rect = () => {
      const r = document
        .querySelector('[data-stream-id]')!
        .getBoundingClientRect();
      return { x: r.x, width: r.width };
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
  expect(samples.at(-1)!.x).toBeLessThan(samples[0].x);
  expect(
    samples.some((s) => s.x < samples[0].x - 1 && s.x > samples.at(-1)!.x + 1),
  ).toBe(true);
  expect(samples.every((s) => s.width > 0)).toBe(true);
  await page.setViewportSize({ width: 480, height: 850 });
  for (const name of ['YouTube', 'Twitch', 'Kick']) {
    await page.getByRole('button', { name, exact: true }).click();
    await expect(page.locator('.provider-surface')).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
});
