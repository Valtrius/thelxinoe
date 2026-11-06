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
// Holds the outgoing copy until a test's assertions finish, even when the
// runner takes longer than the animation to return from the click.
const holdLeave = (page: Page) =>
  page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      const root = this.getRootNode();
      if (
        root instanceof ShadowRoot &&
        root.host.hasAttribute('data-page-leaving')
      ) {
        animation.pause();
        (window as unknown as { heldLeave: Animation }).heldLeave = animation;
        Element.prototype.animate = animate;
      }
      return animation;
    };
  });
const releaseLeave = (page: Page) =>
  page.evaluate(() =>
    (window as unknown as { heldLeave: Animation }).heldLeave.play(),
  );

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
  await holdLeave(page);
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  // The copy exists from the click until the leave animation ends.
  const outgoing = page.locator('[data-page-leaving][aria-hidden="true"]');
  await expect(outgoing).toHaveCount(1);
  await page.screenshot({ path: info.outputPath('movies-leaving-home.png') });
  await releaseLeave(page);
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
    ['Jobs', 'Scan library'],
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

test('the music lists change under a toolbar that stays, requesting the next list at the click', async ({
  page,
}, info) => {
  await page.goto('/#library/music');
  await ready(page);
  await page.evaluate(() => {
    const phases: (string | undefined)[] = [];
    (window as unknown as { listPhases: typeof phases }).listPhases = phases;
    const original = window.fetch;
    window.fetch = (input, init) => {
      const url = String(input);
      if (url.includes('kind=album') && !url.includes('limit=0'))
        phases.push(
          document.querySelector<HTMLElement>('[data-page-content]')?.dataset
            .pagePhase,
        );
      return original(input, init);
    };
  });
  await holdLeave(page);
  const lists = page.getByRole('group', { name: 'Music lists' });
  const albums = lists.getByRole('button', { name: /^Albums/ });
  const artists = lists.getByRole('button', { name: /^Artists/ });
  await expect(artists).toHaveAttribute('aria-pressed', 'true');
  await albums.click();
  const outgoing = page.locator('[data-page-leaving][aria-hidden="true"]');
  await expect(outgoing).toHaveCount(1);
  await expect(page.locator('[data-page-region="music"]')).toHaveCSS(
    'opacity',
    '0',
  );
  await expect(lists).toBeVisible();
  await expect(pageContent(page)).toHaveCSS('opacity', '1');
  await page.screenshot({ path: info.outputPath('music-list-leaving.png') });
  await releaseLeave(page);
  await ready(page);
  await expect(outgoing).toHaveCount(0);
  await expect(albums).toHaveAttribute('aria-pressed', 'true');
  await expect(albums).toBeFocused();
  await expect(page).toHaveURL(/#library\/music\?view=albums$/);
  const phases = await page.evaluate(
    () =>
      (window as unknown as { listPhases: (string | undefined)[] }).listPhases,
  );
  expect(phases[0]).toBe('leaving');
  // An album's Back button returns to the list it was opened from.
  await page.locator('article.library-tile .card-primary').first().click();
  await ready(page);
  await page
    .getByRole('button', { name: 'Back to Music', exact: true })
    .click();
  await ready(page);
  await expect(page).toHaveURL(/#library\/music\?view=albums$/);
  await expect(albums).toHaveAttribute('aria-pressed', 'true');
});

test('the Music page opens on the list last chosen, until another is chosen', async ({
  page,
}) => {
  const listed: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (
      url.pathname === '/api/v1/catalog' &&
      url.searchParams.get('limit') !== '0'
    )
      listed.push(url.searchParams.get('kind') ?? '');
  });
  await page.goto('/#library/music');
  await ready(page);
  const lists = page.getByRole('group', { name: 'Music lists' });
  const albums = lists.getByRole('button', { name: /^Albums/ });
  const artists = lists.getByRole('button', { name: /^Artists/ });
  await albums.click();
  await ready(page);
  await expect(albums).toHaveAttribute('aria-pressed', 'true');
  const navigation = page.getByRole('navigation', { name: 'Main navigation' });
  await navigation.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  await navigation.getByRole('link', { name: 'Music', exact: true }).click();
  await ready(page);
  await expect(page).toHaveURL(/#library\/music$/);
  await expect(albums).toHaveAttribute('aria-pressed', 'true');
  // A new visit reads only that list, not the default one first.
  await expect
    .poll(async () => {
      const saved = await page.request.get('/api/v1/me/appearance');
      return (await saved.json()).provider_preferences?.['music-view'];
    })
    .toBe('albums');
  listed.length = 0;
  await page.reload();
  await ready(page);
  await expect(albums).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('article.library-tile').first()).toBeVisible();
  expect(listed).toEqual(['album']);
  // Going back to the first list is remembered as well.
  await artists.click();
  await ready(page);
  await expect(artists).toHaveAttribute('aria-pressed', 'true');
  await navigation.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  await navigation.getByRole('link', { name: 'Music', exact: true }).click();
  await ready(page);
  await expect(artists).toHaveAttribute('aria-pressed', 'true');
});

test('a Music list keeps its sort after a reload', async ({ page }) => {
  await page.goto('/#library/music');
  await ready(page);
  const sort = page.getByRole('button', { name: 'Sort artists', exact: true });
  const descending = page.getByRole('button', {
    name: 'Name descending',
    exact: true,
  });
  await sort.click();
  await descending.click();
  await expect(descending).toHaveAttribute('aria-pressed', 'true');
  await expect
    .poll(async () => {
      const saved = await page.request.get('/api/v1/me/appearance');
      return (await saved.json()).provider_preferences?.['music-sort'];
    })
    .toContain('desc');
  await page.reload();
  await ready(page);
  await sort.click();
  await expect(descending).toHaveAttribute('aria-pressed', 'true');
});

test('a slow click on a music list does not use up its animations', async ({
  page,
}) => {
  await page.goto('/#library/music');
  await ready(page);
  const lists = page.getByRole('group', { name: 'Music lists' });
  const artists = lists.getByRole('button', { name: /^Artists/ });
  const distance = (await artists.boundingBox())!.width;
  type Frame = { translate: number; leaving: boolean };
  await page.evaluate(() => {
    const frames: Frame[] = [];
    (window as unknown as { listFrames: Frame[] }).listFrames = frames;
    const highlight = document.querySelector<HTMLElement>(
      '[aria-label="Music lists"] [data-choice-selection]',
    )!;
    const sample = () => {
      frames.push({
        translate: parseFloat(getComputedStyle(highlight).translate) || 0,
        leaving: !!document.querySelector('[data-page-leaving]'),
      });
      if (frames.length < 60) requestAnimationFrame(sample);
    };
    document.addEventListener(
      'click',
      () => requestAnimationFrame(sample),
      true,
    );
    // Outlasts the animations, after the app's handler and before any frame.
    document.addEventListener('click', () => {
      const end = performance.now() + 400;
      while (performance.now() < end);
    });
  });
  await lists.getByRole('button', { name: /^Albums/ }).click();
  await ready(page);
  const frames = await page.evaluate(
    () => (window as unknown as { listFrames: Frame[] }).listFrames,
  );
  expect(Math.abs(frames[0].translate)).toBeGreaterThan(distance * 0.9);
  expect(frames.filter((frame) => frame.leaving).length).toBeGreaterThan(2);
});

test('the choice highlight fills its button up to the separator at fractional widths', async ({
  page,
}) => {
  await page.goto('/#library/music');
  await ready(page);
  await page.addStyleTag({
    content:
      '[data-page-content] [aria-label^="Artists"] { width: 90.5px; } [data-page-content] [aria-label^="Albums"] { width: 80.5px; } [data-page-content] [aria-label^="Tracks"] { width: 70.5px; }',
  });
  const lists = page.getByRole('group', { name: 'Music lists' });
  const highlight = lists.locator('[data-choice-selection]');
  for (const [name, separator] of [
    ['Artists', 1],
    ['Albums', 1],
    ['Tracks', 0],
  ] as const) {
    const button = lists.getByRole('button', { name: new RegExp(`^${name}`) });
    await button.click();
    await ready(page);
    await expect
      .poll(async () => {
        const selected = (await button.boundingBox())!;
        const selection = (await highlight.boundingBox())!;
        return (
          Math.abs(selected.x - selection.x) +
          Math.abs(selected.width - separator - selection.width)
        );
      })
      .toBeLessThan(0.05);
  }
});

test('opening the app again brings the player back paused, where Play carries on, until it is closed', async ({
  page,
}) => {
  const started: { position?: number }[] = [];
  page.on('request', (request) => {
    if (
      request.method() === 'POST' &&
      new URL(request.url()).pathname === '/api/v1/playback'
    )
      started.push(request.postDataJSON());
  });
  await page.goto('/#library/music?view=tracks');
  await ready(page);
  const dock = page.getByRole('region', { name: 'Music player' });
  const pause = dock.getByRole('button', { name: 'Pause', exact: true });
  const play = dock.getByRole('button', { name: 'Play', exact: true });
  const position = () =>
    dock
      .getByLabel('Music position')
      .evaluate((input: HTMLInputElement) => Number(input.value));
  const track = page.getByRole('button', { name: /^Play / }).first();
  const title = (await track.getAttribute('aria-label'))!.slice('Play '.length);
  await track.click();
  await expect(pause).toBeVisible();
  await expect.poll(position).toBeGreaterThan(1);
  await pause.click();
  await expect(play).toBeVisible();
  const left = await position();

  started.length = 0;
  await page.reload();
  await ready(page);
  await expect(play).toBeEnabled();
  await expect(dock).toContainText(title);
  expect(await position()).toBeCloseTo(left, 0);
  expect(started).toEqual([]);

  await play.click();
  await expect(pause).toBeVisible();
  await expect.poll(() => started.length).toBe(1);
  expect(started[0].position).toBeCloseTo(left, 0);
  await expect.poll(position).toBeGreaterThan(left);

  await dock.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect(dock).toHaveCount(0);
  await page.reload();
  await ready(page);
  await expect(dock).toHaveCount(0);
});

test('the music dock slides in and out on the animation clock and comes back for a track chosen as it leaves', async ({
  page,
}) => {
  type Dock = { slides: number[]; holdNext: boolean };
  await page.addInitScript(() => {
    const dock: Dock = { slides: [], holdNext: false };
    (window as unknown as { dock: Dock }).dock = dock;
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      const duration = (options as KeyframeAnimationOptions).duration;
      // Svelte's first animation on an intro only holds the starting frame.
      if (
        Array.isArray(keyframes) &&
        keyframes.some((frame) => frame.gridTemplateRows !== undefined) &&
        duration
      ) {
        dock.slides.push(Number(duration));
        if (dock.holdNext) animation.pause();
        dock.holdNext = false;
      }
      return animation;
    };
  });
  await page.goto('/#library/music?view=tracks');
  await ready(page);
  const dock = page.getByRole('region', { name: 'Music player' });
  const pause = dock.getByRole('button', { name: 'Pause', exact: true });
  const close = dock.getByRole('button', { name: 'Close player', exact: true });
  const play = page.getByRole('button', { name: /^Play / }).first();
  const slides = () =>
    page.evaluate(
      () => (window as unknown as { dock: Dock }).dock.slides.length,
    );
  const workspace = page.locator('.workspace-scroll');
  const height = async () => (await workspace.boundingBox())!.height;
  const fullHeight = await height();

  await play.click();
  await expect(pause).toBeVisible();
  expect(await slides()).toBe(1);
  const [duration, clock] = await page.evaluate(() => [
    (window as unknown as { dock: Dock }).dock.slides[0],
    getComputedStyle(document.documentElement).getPropertyValue(
      '--animation-duration',
    ),
  ]);
  expect(duration).toBeGreaterThan(0);
  expect(`${duration}ms`).toBe(clock);
  // The dock's height comes out of the page rather than covering it.
  const dockHeight = (await dock.boundingBox())!.height;
  await expect.poll(height).toBeCloseTo(fullHeight - dockHeight, 0);

  await close.click();
  await expect.poll(slides).toBe(2);
  await expect(dock).toHaveCount(0);
  await expect.poll(height).toBeCloseTo(fullHeight, 0);

  await play.click();
  await expect(pause).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { dock: Dock }).dock.holdNext = true;
  });
  await close.click();
  await expect.poll(slides).toBe(4);
  await play.click();
  await expect(pause).toBeVisible();
  await expect.poll(height).toBeCloseTo(fullHeight - dockHeight, 0);
});
