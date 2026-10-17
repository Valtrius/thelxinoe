import { test, expect, type Page } from '@playwright/test';
import { responseGate } from './helpers/response-gate';

test.use({ video: 'on' });
test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});
test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

const pageContent = (page: Page) => page.locator('[data-page-content]');
const ready = async (page: Page) => {
  await expect(pageContent(page)).toHaveAttribute('data-page-phase', 'idle');
  await expect(pageContent(page)).toHaveCSS('opacity', '1');
};

async function recordFrames(page: Page) {
  await page.evaluate(() => {
    const frames: {
      phase: string | undefined;
      opacity: number;
      scale: number;
      sidebarX?: number;
      sidebarScale?: number;
    }[] = [];
    (window as unknown as { pageFrames: typeof frames }).pageFrames = frames;
    const start = performance.now();
    const sample = () => {
      const root = document.querySelector<HTMLElement>('[data-page-content]');
      if (root) {
        const style = getComputedStyle(root);
        const matrix = new DOMMatrixReadOnly(style.transform);
        const sidebar = root.querySelector<HTMLElement>(
          '[data-youtube-watchlist-frame]',
        );
        const sideMatrix = sidebar
          ? new DOMMatrixReadOnly(getComputedStyle(sidebar).transform)
          : null;
        frames.push({
          phase: root.dataset.pagePhase,
          opacity: Number(style.opacity),
          scale: matrix.a,
          sidebarX: sideMatrix?.e,
          sidebarScale: sideMatrix?.a,
        });
      }
      if (performance.now() - start < 1800) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
}

test('all page families keep delayed content hidden and animate both sides of navigation', async ({
  page,
}, info) => {
  await page.goto('/#home');
  await ready(page);
  const evidence = [];
  for (const [name, endpoint] of [
    ['Discover', '**/api/v1/seerr/discover/trending'],
    ['Movies', '**/api/v1/catalog?**'],
    ['Shows', '**/api/v1/catalog?**'],
    ['Music', '**/api/v1/catalog?**'],
    ['Playlists', '**/api/v1/playlists'],
    ['YouTube', '**/api/v1/online/youtube/browse'],
    ['Twitch', '**/api/v1/online/twitch/feed'],
    ['Kick', '**/api/v1/online/kick'],
    ['Statistics', '**/api/v1/me/statistics?**'],
    ['Settings', '**/api/v1/me/preferences'],
    ['Home', '**/api/v1/me/home'],
  ]) {
    const gate = responseGate();
    let requested = false;
    await page.route(endpoint, async (route) => {
      requested = true;
      await gate.promise;
      await route.fallback();
    });
    try {
      await recordFrames(page);
      await page.getByRole('link', { name, exact: true }).click();
      await expect.poll(() => requested, `${name} starts loading`).toBe(true);
      await expect(pageContent(page)).toHaveCSS('opacity', '0');
      await page.screenshot({
        path: info.outputPath(`${name.toLowerCase()}-hidden.png`),
      });
      gate.release();
      await ready(page);
      const frames = await page.evaluate(
        () =>
          (
            window as unknown as {
              pageFrames: { phase?: string; opacity: number; scale: number }[];
            }
          ).pageFrames,
      );
      expect(
        frames.some(
          (frame) =>
            frame.phase === 'leaving' &&
            frame.opacity > 0 &&
            frame.opacity < 1 &&
            frame.scale < 1,
        ),
        `${name} has an outgoing fade and scale`,
      ).toBe(true);
      expect(
        frames.some(
          (frame) =>
            frame.phase === 'entering' &&
            frame.opacity > 0 &&
            frame.opacity < 1,
        ),
        `${name} has an incoming fade`,
      ).toBe(true);
      evidence.push({ name, frames });
    } finally {
      gate.release();
      await page.unroute(endpoint);
    }
  }
  await info.attach('navigation-frames.json', {
    body: JSON.stringify(evidence, null, 2),
    contentType: 'application/json',
  });
});

test('visible artwork finishes loading before the replacement page fades in', async ({
  page,
}, info) => {
  await page.goto('/#home');
  await ready(page);
  const artwork = responseGate();
  await page.route('**/mock-art/movie/0', async (route) => {
    await artwork.promise;
    await route.continue();
  });
  try {
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    await expect(
      page.locator('article[data-layout-key="library:movie-0"]'),
    ).toBeAttached();
    await expect(pageContent(page)).toHaveCSS('opacity', '0');
    artwork.release();
    await ready(page);
    expect(
      await page
        .locator('article.library-tile img')
        .first()
        .evaluate(
          (image: HTMLImageElement) => image.complete && image.naturalWidth > 0,
        ),
    ).toBe(true);
    await page.screenshot({
      path: info.outputPath('movies-after-artwork.png'),
    });
  } finally {
    artwork.release();
  }
});

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`settings navigation and service tabs stay still while their content transitions at ${viewport.width}px`, async ({
    page,
  }, info) => {
    await page.setViewportSize(viewport);
    await page.goto('/#settings/account');
    await ready(page);
    const navigation = page.getByRole('navigation', {
      name: 'Settings navigation',
    });
    const evidence = [];
    for (const [name, region, navigationLabel] of [
      ['Playback', 'settings', 'Settings navigation'],
      ['Media services', 'settings', 'Settings navigation'],
      ['Radarr', 'service', 'Select service'],
      ['Sonarr', 'service', 'Select service'],
    ]) {
      const link = page
        .getByRole('navigation', { name: navigationLabel })
        .getByRole('link', { name, exact: true });
      await link.scrollIntoViewIfNeeded();
      await page.evaluate(
        ({ region, navigationLabel }) => {
          const frames: {
            phase?: string;
            opacity: number;
            scale: number;
            navigationStill: boolean;
          }[] = [];
          (window as unknown as { regionFrames: typeof frames }).regionFrames =
            frames;
          let started = false;
          const sample = () => {
            const root = document.querySelector<HTMLElement>(
              '[data-page-content]',
            )!;
            const content = root.querySelector<HTMLElement>(
              `[data-page-region="${region}"]`,
            );
            const navigation = root.querySelector<HTMLElement>(
              `nav[aria-label="${navigationLabel}"]`,
            );
            let navigationStill = !!navigation;
            for (
              let element = navigation;
              element;
              element = element.parentElement
            ) {
              const style = getComputedStyle(element);
              navigationStill &&=
                style.opacity === '1' && style.transform === 'none';
              if (element === root) break;
            }
            if (content) {
              const style = getComputedStyle(content);
              frames.push({
                phase: root.dataset.pagePhase,
                opacity: Number(style.opacity),
                scale: new DOMMatrixReadOnly(style.transform).a,
                navigationStill,
              });
            }
            started ||= root.dataset.pagePhase !== 'idle';
            if (!started || root.dataset.pagePhase !== 'idle')
              requestAnimationFrame(sample);
          };
          requestAnimationFrame(sample);
        },
        { region, navigationLabel },
      );
      await link.click();
      await ready(page);
      const frames = await page.evaluate(
        () =>
          (
            window as unknown as {
              regionFrames: {
                phase?: string;
                opacity: number;
                scale: number;
                navigationStill: boolean;
              }[];
            }
          ).regionFrames,
      );
      expect(frames.length).toBeGreaterThan(0);
      expect(
        frames.every((frame) => frame.navigationStill),
        `${name} keeps navigation fully visible and unscaled`,
      ).toBe(true);
      for (const phase of ['leaving', 'entering'])
        expect(
          frames.some(
            (frame) =>
              frame.phase === phase &&
              frame.opacity > 0 &&
              frame.opacity < 1 &&
              frame.scale < 1,
          ),
          `${name} content ${phase}`,
        ).toBe(true);
      await expect(link).toBeFocused();
      evidence.push({ name, frames });
    }
    const devices = responseGate();
    await page.route('**/api/v1/auth/sessions', async (route) => {
      await devices.promise;
      await route.fallback();
    });
    try {
      await navigation
        .getByRole('link', { name: 'Devices', exact: true })
        .click();
      await expect(page.locator('[data-page-region="settings"]')).toHaveCSS(
        'opacity',
        '0',
      );
      await expect(pageContent(page)).toHaveCSS('opacity', '1');
      await navigation
        .getByRole('link', { name: 'Account', exact: true })
        .click();
      await ready(page);
      await expect(
        page.getByLabel('New password', { exact: true }),
      ).toBeVisible();
    } finally {
      devices.release();
    }
    await info.attach('settings-transition-frames.json', {
      body: JSON.stringify(evidence, null, 2),
      contentType: 'application/json',
    });
    await page.screenshot({
      path: info.outputPath('settings-navigation-retained.png'),
    });
  });
}

test('settings sections, service tabs and detail pages also transition without losing scroll or search focus', async ({
  page,
}, info) => {
  await page.goto('/#settings/account');
  await ready(page);
  for (const [name, content] of [
    ['Playback', 'Default quality'],
    ['Online accounts', 'Preview admin'],
    ['Devices', 'Living room desktop'],
    ['Server', '2026.09.30'],
    ['Episode analysis', 'Beyond the Horizon · First Contact'],
    ['Provider applications', 'Daily API budget'],
    ['Media services', 'Seerr'],
    ['Auto-delete', 'Video downloads'],
    ['Backups', 'thelxinoe-preview-0.tar.age'],
    ['People', 'Alex'],
    ['Activity', 'Library scan'],
    ['Audit', 'Created a user'],
  ]) {
    await page
      .getByRole('navigation', { name: 'Settings navigation' })
      .getByRole('link', { name, exact: true })
      .click();
    await ready(page);
    await expect(
      page
        .getByText(content, { exact: false })
        .filter({ visible: true })
        .first(),
    ).toBeVisible();
    if (name === 'Media services') {
      for (const service of ['Radarr', 'Sonarr', 'NZBGet', 'Seerr']) {
        const link = page
          .getByRole('navigation', { name: 'Select service' })
          .getByRole('link', { name: service, exact: true });
        await link.click();
        await ready(page);
        await expect(link).toHaveAttribute('aria-current', 'true');
        await expect(link).toBeFocused();
      }
    }
  }
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  const search = page.getByPlaceholder('Search your movies');
  await search.fill('Beyond');
  await expect(search).toBeFocused();
  await ready(page);
  await expect(page.locator('article.library-tile')).toHaveCount(2);
  await page
    .locator('article.library-tile')
    .first()
    .locator('.card-primary')
    .click();
  await ready(page);
  await expect(
    page.getByRole('heading', {
      level: 1,
      name: 'Beyond the Horizon',
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Back to movies', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: info.outputPath('transitioned-library-detail.png'),
  });
});

test('a debounced search from the outgoing library cannot replace the selected page', async ({
  page,
}, info) => {
  await page.clock.install();
  await page.goto('/#library/movies');
  await ready(page);
  await page.getByPlaceholder('Search your movies').fill('Beyond');
  await page.clock.runFor(150);
  await page.getByRole('link', { name: 'Shows', exact: true }).click();
  await page.clock.runFor(300);
  await ready(page);
  await expect(page).toHaveURL(/#library\/shows$/);
  await expect(page.getByPlaceholder('Search your shows')).toHaveValue('');
  await page.screenshot({
    path: info.outputPath('outgoing-search-keeps-shows.png'),
  });
});

test('the latest navigation wins during loading and error pages still reveal with reduced motion', async ({
  page,
}, info) => {
  await page.goto('/#home');
  await ready(page);
  const movies = responseGate();
  await page.route('**/api/v1/catalog?kind=movie**', async (route) => {
    await movies.promise;
    await route.fallback();
  });
  try {
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    await expect(pageContent(page)).toHaveCSS('opacity', '0');
    await page.getByRole('link', { name: 'Music', exact: true }).click();
    await ready(page);
    await expect(
      page.locator('article[data-layout-key^="library:artist-"]'),
    ).toHaveCount(24);
    movies.release();
    await expect(
      page.locator('article[data-layout-key^="library:movie-"]'),
    ).toHaveCount(0);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.route('**/api/v1/me/statistics?**', (route) =>
      route.fulfill({
        status: 503,
        json: { error: { message: 'Statistics unavailable' } },
      }),
    );
    await page.getByRole('link', { name: 'Statistics', exact: true }).click();
    await ready(page);
    await expect(
      page.getByText('Statistics unavailable', { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('transition-error-recovery.png'),
    });
  } finally {
    movies.release();
  }
});

test('the saved YouTube watchlist sidebar slides from the right without scaling on page entry', async ({
  page,
}, info) => {
  await page.goto('/#home');
  await ready(page);
  await page.getByRole('link', { name: 'YouTube', exact: true }).click();
  await ready(page);
  const open = page.getByRole('button', {
    name: 'Open watchlists',
    exact: true,
  });
  if (await open.isVisible()) await open.click();
  await expect(page.locator('[data-youtube-watchlist-frame]')).toHaveAttribute(
    'aria-hidden',
    'false',
  );
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  await recordFrames(page);
  await page.getByRole('link', { name: 'YouTube', exact: true }).click();
  await ready(page);
  const frames = await page.evaluate(
    () =>
      (
        window as unknown as {
          pageFrames: {
            phase?: string;
            opacity: number;
            sidebarX?: number;
            sidebarScale?: number;
          }[];
        }
      ).pageFrames,
  );
  const entering = frames.filter((frame) => frame.phase === 'entering');
  expect(
    entering.some(
      (frame) =>
        (frame.sidebarX ?? 0) > 1 && frame.opacity > 0 && frame.opacity < 1,
    ),
  ).toBe(true);
  expect(entering.every((frame) => frame.sidebarScale === 1)).toBe(true);
  await info.attach('youtube-sidebar-entry.json', {
    body: JSON.stringify(frames, null, 2),
    contentType: 'application/json',
  });
  await page.screenshot({
    path: info.outputPath('youtube-sidebar-entered.png'),
  });
});
