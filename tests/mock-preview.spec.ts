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
    await expect(page.locator('article.library-tile')).toHaveCount(24);
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

  test('watched movies, episodes, seasons and completed shows follow the shared fade preference', async ({
    page,
  }, info) => {
    await page.goto('/#home');
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    const movie = page.locator('article[data-layout-key="library:movie-0"]');
    await movie.locator('.card-primary').click();
    await page
      .getByRole('button', { name: 'Mark watched', exact: true })
      .click();
    await page.getByRole('button', { name: 'Close', exact: true }).click();
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
    const season = page.locator('article[data-layout-key="library:season-0"]');
    await expect(season).toHaveCSS('opacity', '0.55');
    await season.locator('.card-primary').click();
    const episode = page.locator(
      'article[data-layout-key="library:episode-0"]',
    );
    await page.mouse.move(0, 0);
    await expect(episode).toHaveCSS('opacity', '0.55');
    await episode.locator('.card-primary').click();
    await page
      .getByRole('button', { name: 'Mark unwatched', exact: true })
      .click();
    await page.getByRole('button', { name: 'Close', exact: true }).click();
    await page.mouse.move(0, 0);
    await expect(episode).toHaveCSS('opacity', '1');
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
    'Playlists',
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

test('mock Home, Discover and request details work without external services', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  await page.goto('/#home');
  for (const name of [
    'Continue watching',
    'Next up',
    'YouTube subscriptions',
    'Live now',
    'Watch later',
    'Favorites',
  ])
    await expect(
      page.getByRole('heading', { name, exact: true }),
    ).toBeVisible();
  await expect(page.getByText(/live status is out of date/i)).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('home.png'), fullPage: true });
  await page.goto('/#discover');
  await expect(
    page.getByRole('heading', { name: 'Trending', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: info.outputPath('discover.png'),
    fullPage: true,
  });
  await page.goto('/movie/11');
  await expect(
    page.getByRole('heading', { name: 'Beyond the Horizon', exact: true }),
  ).toBeVisible();
  await expect(page.locator('article img').first()).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator('article img')
        .first()
        .evaluate(
          (image: HTMLImageElement) => image.complete && image.naturalWidth > 0,
        ),
    )
    .toBe(true);
  await page.screenshot({ path: info.outputPath('movie.png'), fullPage: true });
  await page.getByRole('button', { name: 'Request', exact: true }).click();
  await expect(
    page
      .locator('article')
      .getByRole('button', { name: 'Requested', exact: true }),
  ).toBeDisabled();
  await page.reload();
  await expect(
    page
      .locator('article')
      .getByRole('button', { name: 'Requested', exact: true }),
  ).toBeDisabled();
  await page.goto('/tv/22');
  await expect(
    page.getByRole('heading', { name: 'Northern Lights', exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: info.outputPath('tv.png'), fullPage: true });
  await expect(
    page.getByRole('button', { name: 'Request', exact: true }),
  ).toBeEnabled();
  await expect(page.getByRole('checkbox', { name: /Season 1/ })).toBeDisabled();
  await page.getByRole('button', { name: 'Request', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: /Season 2/ })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: /Season 3/ })).toBeDisabled();
  await page.goto('/#requests');
  await expect(
    page.getByText('Beyond the Horizon', { exact: true }).first(),
  ).toBeVisible();
  await page.screenshot({
    path: info.outputPath('requests.png'),
    fullPage: true,
  });
  expect(errors).toEqual([]);
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});

test('every admin settings panel and service tab has populated mocks', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  const panels = [
    ['settings/account', '6 columns'],
    ['settings/playback', 'Default quality'],
    ['settings/online', 'Preview admin'],
    ['settings/devices', 'Living room desktop'],
    ['admin/server', '2026.09.30'],
    ['admin/analysis', 'Beyond the Horizon · First Contact'],
    ['admin/providers', 'Daily API budget'],
    ['admin/services', 'Seerr'],
    ['admin/retention', 'Video downloads'],
    ['admin/backups', 'thelxinoe-preview-0.tar.age'],
    ['admin/people', 'Alex'],
    ['admin/jobs', 'Library scan'],
    ['admin/audit', 'Created a user'],
  ];
  for (const [route, content] of panels) {
    await page.goto(`/#${route}`);
    await expect(
      page.getByRole('navigation', { name: 'Settings navigation' }),
    ).toBeVisible();
    await expect(
      page.locator('main').getByText(content).filter({ visible: true }).first(),
    ).toBeVisible();
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath(`settings-${route.split('/')[1]}.png`),
      fullPage: true,
    });
    await expect(
      page.getByText(
        /Unexpected fixture request|No mock for|Mock request failed/,
      ),
    ).toHaveCount(0);
  }
  for (const service of [
    'seerr',
    'recyclarr',
    'radarr',
    'sonarr',
    'lidarr',
    'bazarr',
    'prowlarr',
    'nzbget',
  ]) {
    await page.goto(`/#admin/services/${service}`);
    await expect(
      page.getByRole('navigation', { name: 'Select service' }),
    ).toBeVisible();
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath(`service-${service}.png`),
      fullPage: true,
    });
  }
  expect(errors).toEqual([]);
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});

test('the preview switches between the admin and user views', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  const views = page.getByRole('group', { name: 'Preview view' });
  const settings = page.getByRole('navigation', {
    name: 'Settings navigation',
  });
  async function view(name: 'Admin' | 'User') {
    const reloaded = page.waitForEvent('load');
    await views.getByRole('button', { name, exact: true }).click();
    await reloaded;
    await expect(
      views.getByRole('button', { name, exact: true }),
    ).toHaveAttribute('aria-pressed', 'true');
  }

  await page.goto('/#settings/account');
  await expect(
    views.getByRole('button', { name: 'Admin', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await expect(settings.getByText('Administration')).toBeVisible();

  await view('User');
  await expect(page.getByText('Member', { exact: true })).toBeVisible();
  await expect(settings.getByText('Personal')).toBeVisible();
  await expect(settings.getByText('Administration')).toHaveCount(0);
  expect((await page.request.get('/api/v1/admin/settings')).status()).toBe(403);
  await page.goto('/#admin/server');
  await expect(
    page.getByRole('heading', { name: 'Account', level: 1 }),
  ).toBeVisible();
  for (const route of [
    'home',
    'discover',
    'statistics',
    'settings/playback',
    'settings/online',
    'settings/devices',
  ]) {
    await page.goto(`/#${route}`);
    await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  }
  await page.goto('/#discover');
  await page.getByRole('button', { name: 'My requests', exact: true }).click();
  await expect(page.locator('[aria-busy="true"]')).toHaveCount(0);
  const requests = await (
    await page.request.get('/api/v1/seerr/requests')
  ).json();
  expect(requests.results.length).toBeGreaterThan(0);
  for (const request of requests.results)
    expect(request.requestedBy.username).toBe('Alex');
  await page.screenshot({
    path: info.outputPath('user-view-requests.png'),
    fullPage: true,
  });

  await view('Admin');
  await page.goto('/#settings/account');
  await expect(settings.getByText('Administration')).toBeVisible();
  expect(errors).toEqual([]);
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});

test('mock settings, service controls and playback retain changes', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  await page.goto('/#settings/account');
  await page
    .getByRole('textbox', { name: 'Passkey name', exact: true })
    .fill('Preview laptop');
  await page.getByRole('button', { name: 'Add passkey', exact: true }).click();
  await expect(
    page.getByRole('button', {
      name: 'Remove passkey Preview laptop',
      exact: true,
    }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('button', {
      name: 'Remove passkey Preview laptop',
      exact: true,
    }),
  ).toBeVisible();
  await page.goto('/#settings/playback');
  await page
    .getByRole('combobox', { name: /Default quality/ })
    .selectOption('720p');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/playback/preferences')).json())
          .quality,
    )
    .toBe('720p');
  await page.reload();
  await expect(
    page.getByRole('combobox', { name: /Default quality/ }),
  ).toHaveValue('720p');
  await page.goto('/#admin/services/nzbget');
  await page.getByRole('button', { name: 'Pause all', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Resume all', exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Resume all', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Resume all', exact: true }).click();
  await page
    .getByRole('button', { name: 'Pause Beyond the Horizon', exact: true })
    .click();
  await expect(
    page.getByRole('button', {
      name: 'Resume Beyond the Horizon',
      exact: true,
    }),
  ).toBeVisible();
  await page.goto('/#admin/services/radarr');
  await page
    .getByRole('region', { name: 'Download clients', exact: true })
    .getByRole('button', { name: 'Disconnect', exact: true })
    .click();
  await expect(
    page.getByText('Available to connect', { exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByText('Available to connect', { exact: true }),
  ).toBeVisible();
  await page.goto('/#admin/services/recyclarr');
  await page.getByRole('button', { name: 'Edit YAML', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  await page.screenshot({
    path: info.outputPath('yaml-editor.png'),
    fullPage: true,
  });
  await page
    .getByRole('button', { name: 'Close YAML editor', exact: true })
    .click();
  await page.goto('/#library/movies/movie-0');
  await page
    .getByRole('button', { name: 'Play Theatrical', exact: true })
    .click();
  await expect
    .poll(() =>
      page
        .locator('video')
        .evaluate(
          (video: HTMLVideoElement) =>
            video.readyState >= 2 && video.currentTime > 0,
        ),
    )
    .toBe(true);
  await page.screenshot({
    path: info.outputPath('player.png'),
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  expect(errors).toEqual([]);
  expect(
    (await (await page.request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
});

test('library hierarchy, playlist and watchlist edits, online feeds and statistics work', async ({
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
    'playlists',
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
  await page.goto('/#playlists');
  await page.getByRole('button', { name: 'New playlist', exact: true }).click();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
  await page
    .getByRole('textbox', { name: 'Playlist name', exact: true })
    .fill('Preview tracks');
  await page
    .getByRole('textbox', { name: 'Find tracks', exact: true })
    .fill('Daybreak');
  await page
    .getByRole('button', { name: 'Search tracks', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Add Daybreak', exact: true })
    .first()
    .click();
  await page
    .getByRole('button', { name: 'Save playlist', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Preview tracks', exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('heading', { name: '1 tracks', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: info.outputPath('saved-playlist.png'),
    fullPage: true,
  });
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
  await page
    .getByRole('button', { name: 'Watchlist settings', exact: true })
    .click();
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
  ).toContainText('7 columns');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .poster_columns,
    )
    .toBe(7);
  await page.reload();
  await expect(
    page.getByRole('group', { name: 'Poster density' }),
  ).toContainText('7 columns');
  expect(errors).toEqual([]);
  const diagnostic = await page.request.get('/__mock/diagnostics');
  expect((await diagnostic.json()).unhandled).toEqual([]);
});
