import { test, expect } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
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
    ['admin/services', 'Automatic request approval'],
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
