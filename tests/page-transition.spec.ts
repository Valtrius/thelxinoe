import { test, expect, type Page } from '@playwright/test';
import { responseGate } from './helpers/response-gate';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});
test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

const pageContent = (page: Page) => page.locator('[data-page-content]');
const pageLoading = (page: Page) =>
  page.getByRole('status', { name: 'Loading page', exact: true });
const ready = async (page: Page) => {
  await expect(pageContent(page)).toHaveAttribute('data-page-phase', 'idle');
  await expect(pageContent(page)).toHaveCSS('opacity', '1');
};

test('the next page revalidates stale data while the previous page is still leaving', async ({
  page,
}, info) => {
  await page.goto('/#home');
  await ready(page);
  // Load the Movies code first, then age its data past the fresh cache window.
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await ready(page);
  await page.clock.setFixedTime(
    new Date((await page.evaluate(() => Date.now())) + 31_000),
  );
  await page.evaluate(() => {
    const phases: (string | undefined)[] = [];
    (window as unknown as { catalogPhases: typeof phases }).catalogPhases =
      phases;
    const original = window.fetch;
    window.fetch = (input, init) => {
      if (String(input).includes('/api/v1/catalog?'))
        phases.push(
          document.querySelector<HTMLElement>('[data-page-content]')?.dataset
            .pagePhase,
        );
      return original(input, init);
    };
  });
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  // The copy exists from the click until the leave animation ends.
  const outgoing = page.locator('[data-page-leaving][aria-hidden="true"]');
  await expect(outgoing).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('movies-leaving-home.png') });
  await ready(page);
  await expect(outgoing).toHaveCount(0);
  const phases = await page.evaluate(
    () =>
      (window as unknown as { catalogPhases: (string | undefined)[] })
        .catalogPhases,
  );
  expect(phases[0]).toBe('leaving');
  await expect(
    page.locator('article[data-layout-key^="library:movie-"]').first(),
  ).toBeVisible();
});

test('a page appears without waiting long for its artwork, which fades in as it arrives', async ({
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
    await ready(page);
    const image = page
      .locator('article[data-layout-key="library:movie-0"] img')
      .first();
    await expect(image).toHaveAttribute('data-arriving', '');
    await expect(image).toHaveCSS('opacity', '0');
    await page.screenshot({
      path: info.outputPath('movies-before-artwork.png'),
    });
    artwork.release();
    await expect(image).not.toHaveAttribute('data-arriving');
    await expect(image).toHaveCSS('opacity', '1');
    expect(
      await image.evaluate(
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

test('the server settings page appears while the administration overview is still loading', async ({
  page,
}, info) => {
  await page.goto('/#settings/account');
  await ready(page);
  const overview = responseGate();
  await page.route('**/api/v1/admin/operations', async (route) => {
    await overview.promise;
    await route.fallback();
  });
  try {
    await page
      .getByRole('navigation', { name: 'Settings navigation' })
      .getByRole('link', { name: 'Server', exact: true })
      .click();
    await ready(page);
    const card = page.getByRole('region', {
      name: 'Administration overview',
    });
    await expect(
      card.getByRole('status', { name: 'Loading administration overview' }),
    ).toBeVisible();
    await expect(pageLoading(page)).toHaveCount(0);
    await expect(
      page.getByRole('heading', { name: 'Display defaults', exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('server-overview-loading.png'),
    });
    overview.release();
    await expect(
      card.getByRole('heading', { name: 'Storage', exact: true }),
    ).toBeVisible();
    await expect(
      card.getByRole('status', { name: 'Loading administration overview' }),
    ).toHaveCount(0);
  } finally {
    overview.release();
  }
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
