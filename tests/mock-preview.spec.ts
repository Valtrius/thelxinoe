import { test, expect } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

test.describe('media card motion', () => {
  test('library and provider cards lift smoothly and zoom their artwork on hover', async ({
    page,
  }, info) => {
    await page.goto('/#home');
    const evidence = [];
    for (const name of [
      'Movies',
      'Shows',
      'Music',
      'YouTube',
      'Twitch',
      'Kick',
    ]) {
      await page.getByRole('link', { name, exact: true }).click();
      await expect(page.locator('[data-page-content]')).toHaveAttribute(
        'data-page-phase',
        'idle',
      );
      const card = page.locator('[data-card-grid] article').first();
      await expect(card).toBeVisible();
      await expect(card.locator('img').first()).toBeVisible();
      await card.evaluate((element) => {
        const samples: { lift: number; zoom: number }[] = [];
        (window as unknown as { hoverSamples: typeof samples }).hoverSamples =
          samples;
        element.addEventListener(
          'pointerenter',
          () => {
            const image = element.querySelector('img')!;
            const sample = () => {
              samples.push({
                lift:
                  Number.parseFloat(
                    getComputedStyle(element).translate.split(' ')[1] ?? '0',
                  ) || 0,
                zoom: Number.parseFloat(getComputedStyle(image).scale) || 1,
              });
              if (samples.length < 30) requestAnimationFrame(sample);
            };
            requestAnimationFrame(sample);
          },
          { once: true },
        );
      });
      await card.hover();
      await expect
        .poll(() =>
          card.evaluate((element) =>
            Number.parseFloat(
              getComputedStyle(element.querySelector('img')!).scale,
            ),
          ),
        )
        .toBeCloseTo(1.02, 3);
      await expect
        .poll(() =>
          card.evaluate((element) => getComputedStyle(element).translate),
        )
        .toBe('0px -2px');
      const samples = await page.evaluate(
        () =>
          (
            window as unknown as {
              hoverSamples: { lift: number; zoom: number }[];
            }
          ).hoverSamples,
      );
      expect(
        samples.some(({ lift }) => lift < -0.05 && lift > -1.95),
        `${name} lift has intermediate frames`,
      ).toBe(true);
      expect(
        samples.some(({ zoom }) => zoom > 1.001 && zoom < 1.019),
        `${name} zoom has intermediate frames`,
      ).toBe(true);
      evidence.push({ name, samples });
      await page.screenshot({
        path: info.outputPath(`hover-${name.toLowerCase()}.png`),
      });
      await page.mouse.move(0, 0);
      await expect
        .poll(() =>
          card.evaluate((element) => ({
            lift:
              Number.parseFloat(
                getComputedStyle(element).translate.split(' ')[1] ?? '0',
              ) || 0,
            zoom:
              Number.parseFloat(
                getComputedStyle(element.querySelector('img')!).scale,
              ) || 1,
          })),
        )
        .toEqual({ lift: 0, zoom: 1 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await card.hover();
      await expect
        .poll(() =>
          card.evaluate((element) => ({
            lift:
              Number.parseFloat(
                getComputedStyle(element).translate.split(' ')[1] ?? '0',
              ) || 0,
            zoom:
              Number.parseFloat(
                getComputedStyle(element.querySelector('img')!).scale,
              ) || 1,
            animating: element
              .getAnimations({ subtree: true })
              .some((animation) => animation.playState === 'running'),
          })),
        )
        .toEqual({ lift: -2, zoom: 1.02, animating: false });
      await page.mouse.move(0, 0);
      await page.emulateMedia({ reducedMotion: 'no-preference' });
    }
    await info.attach('hover-frames.json', {
      body: JSON.stringify(evidence, null, 2),
      contentType: 'application/json',
    });
  });

  test('changing library sections never animates one collection into another', async ({
    page,
  }, info) => {
    await page.goto('/#home');
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    // 24 library movies and 4 Radarr titles that have no files yet.
    await expect(page.locator('article.library-tile')).toHaveCount(28);
    const evidence = [];
    for (const name of [
      'Shows',
      'Music',
      'Movies',
      'Music',
      'Shows',
      'Movies',
    ]) {
      await page.evaluate(() => {
        const samples: {
          transform: string;
          opacity: number;
          key: string | null;
        }[][] = [];
        (
          window as unknown as { collectionSamples: typeof samples }
        ).collectionSamples = samples;
        const sample = () => {
          samples.push(
            Array.from(
              document.querySelectorAll('article.library-tile'),
              (element) => ({
                transform: getComputedStyle(element).transform,
                opacity: Number(getComputedStyle(element).opacity),
                key: element.getAttribute('data-layout-key'),
              }),
            ),
          );
          if (samples.length < 40) requestAnimationFrame(sample);
        };
        requestAnimationFrame(sample);
      });
      await page.getByRole('link', { name, exact: true }).click();
      const kind =
        name === 'Music' ? 'artist' : name === 'Shows' ? 'show' : 'movie';
      await expect(
        page.locator(`article[data-layout-key^="library:${kind}-"]`),
      ).toHaveCount(24);
      await expect
        .poll(() =>
          page.evaluate(
            () =>
              (window as unknown as { collectionSamples: unknown[] })
                .collectionSamples.length,
          ),
        )
        .toBe(40);
      const samples = await page.evaluate(
        () =>
          (
            window as unknown as {
              collectionSamples: {
                transform: string;
                opacity: number;
                key: string | null;
              }[][];
            }
          ).collectionSamples,
      );
      evidence.push({ name, samples });
      expect(
        samples
          .flat()
          .filter(
            (sample) => sample.transform !== 'none' || sample.opacity !== 1,
          ),
      ).toEqual([]);
    }
    await info.attach('collection-switch-frames.json', {
      body: JSON.stringify(evidence, null, 2),
      contentType: 'application/json',
    });
    await page.screenshot({
      path: info.outputPath('library-section-switch.png'),
    });
  });

  test('watched movies, episodes and completed shows follow the shared fade preference', async ({
    page,
  }, info) => {
    await page.goto('/#home');
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    const movie = page.locator('article[data-layout-key="library:movie-0"]');
    await movie.hover();
    await movie
      .getByRole('button', {
        name: 'More actions for Beyond the Horizon',
        exact: true,
      })
      .click();
    await page.getByRole('menuitem', { name: 'Mark watched' }).click();
    await expect(page.getByRole('menu')).toHaveCount(0);
    // The menu returns focus to its trigger, and focused cards never fade.
    await page.evaluate(() => (document.activeElement as HTMLElement).blur());
    await page.mouse.move(0, 0);
    await expect(movie).toHaveCSS('opacity', '0.55');
    await movie.hover();
    await expect(movie).toHaveCSS('opacity', '1');
    await page.mouse.move(0, 0);
    await movie.locator('.card-primary').focus();
    await expect(movie).toHaveCSS('opacity', '1');
    await page.getByRole('link', { name: 'Shows', exact: true }).click();
    for (let i = 0; i < 8; i++)
      expect(
        (
          await page.request.put(`/api/v1/catalog/episode-${i}/state`, {
            data: { watched: true },
          })
        ).ok(),
      ).toBe(true);
    const show = page.locator('article[data-layout-key="library:show-0"]');
    await expect(show).toHaveCSS('opacity', '0.55');
    await show.locator('.card-primary').click();
    const episode = page.getByRole('button', {
      name: 'Watched: First Contact',
      exact: true,
    });
    await expect(episode).toHaveAttribute('aria-pressed', 'true');
    await episode.click();
    await expect(episode).toHaveAttribute('aria-pressed', 'false');
    await page.getByRole('link', { name: 'Shows', exact: true }).click();
    await expect(show).toHaveCSS('opacity', '1');
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await page
      .getByRole('switch', { name: 'Fade watched videos', exact: true })
      .click();
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    await expect(movie).toHaveCSS('opacity', '1');
    await page.reload();
    await expect(movie).toHaveCSS('opacity', '1');
    await page.screenshot({
      path: info.outputPath('watched-movie-preference.png'),
    });
  });
});

test('mobile page headers show each page icon immediately before its name', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#home');
  const menu = page.getByRole('button', { name: 'Open navigation' });
  const drawer = page.getByRole('dialog', { name: 'Navigation menu' });
  const geometry = [];
  for (const name of [
    'Home',
    'Discover',
    'Movies',
    'Shows',
    'Music',
    'YouTube',
    'Twitch',
    'Kick',
    'Statistics',
    'Settings',
  ]) {
    await menu.click();
    await drawer.getByRole('link', { name, exact: true }).click();
    await expect(drawer).toBeHidden();
    const heading = page
      .locator('.page-header')
      .getByRole('heading', { name, exact: true });
    await expect(heading).toBeVisible();
    const icon = heading.locator('svg');
    await expect(icon).toBeVisible();
    const iconBounds = (await icon.boundingBox())!;
    const titleBounds = (await heading
      .getByText(name, { exact: true })
      .boundingBox())!;
    expect(iconBounds.x + iconBounds.width).toBeLessThan(titleBounds.x);
    expect(titleBounds.x - iconBounds.x - iconBounds.width).toBeLessThanOrEqual(
      12,
    );
    expect(
      Math.abs(
        iconBounds.y +
          iconBounds.height / 2 -
          titleBounds.y -
          titleBounds.height / 2,
      ),
    ).toBeLessThanOrEqual(1);
    geometry.push({ name, icon: iconBounds, title: titleBounds });
    if (['Home', 'YouTube', 'Settings'].includes(name))
      await page.screenshot({
        path: info.outputPath(`mobile-header-${name.toLowerCase()}.png`),
      });
  }
  await info.attach('mobile-header-geometry.json', {
    body: JSON.stringify(geometry, null, 2),
    contentType: 'application/json',
  });
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});

test('library hierarchy, watchlist edits, online feeds and statistics work', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  for (const route of [
    'library/movies',
    'library/movies/movie-0',
    'library/shows',
    'library/shows/show-0',
    'library/music',
    'library/music/artist-0',
    'online/youtube',
    'online/twitch',
    'online/kick',
    'statistics',
  ]) {
    await page.goto(`/#${route}`);
    await expect(page.locator('main')).toBeVisible();
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath(`${route.replaceAll('/', '-')}.png`),
      fullPage: true,
    });
  }
  await page.goto('/#online/youtube');
  const openWatchlists = page.getByRole('button', {
    name: 'Open watchlists',
    exact: true,
  });
  await openWatchlists.click();
  await page.getByRole('button', { name: /^Watch Later \d+$/ }).click();
  await page
    .getByPlaceholder('New watchlist', { exact: true })
    .fill('Preview videos');
  await page
    .getByRole('button', { name: 'Create watchlist', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Watchlist settings', exact: true })
    .click();
  const settings = page.getByRole('dialog', {
    name: 'Watchlist settings',
    exact: true,
  });
  await settings
    .getByRole('switch', {
      name: 'Automatically download added videos',
      exact: true,
    })
    .click();
  await settings
    .getByRole('switch', { name: 'Remove videos when watched', exact: true })
    .click();
  // The menu's backdrop covers its button and takes the press that closes it.
  await page
    .getByRole('button', { name: 'Watchlist settings', exact: true })
    .click({ force: true });
  await expect(settings).toBeHidden();
  const card = page.locator('article[data-video-id]').nth(1);
  await card.hover();
  await card.getByRole('button', { name: /^More actions for / }).click();
  await card
    .getByRole('button', { name: 'Add to Preview videos', exact: true })
    .click();
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Preview videos 1', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Watchlist settings', exact: true })
    .click();
  await expect(
    settings.getByRole('switch', {
      name: 'Automatically download added videos',
      exact: true,
    }),
  ).toBeChecked();
  await expect(
    settings.getByRole('switch', {
      name: 'Remove videos when watched',
      exact: true,
    }),
  ).toBeChecked();
  await page.screenshot({
    path: info.outputPath('saved-watchlist.png'),
    fullPage: true,
  });
  await page.goto('/#settings/account');
  await page.getByRole('button', { name: 'Smaller poster cards' }).click();
  await expect(
    page.getByRole('group', { name: 'Poster density' }),
  ).toContainText('10 columns');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .poster_columns,
    )
    .toBe(10);
  await page.reload();
  await expect(
    page.getByRole('group', { name: 'Poster density' }),
  ).toContainText('10 columns');
  expect(errors).toEqual([]);
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});
