import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

test.afterEach(async ({ request }) => {
  expect(
    (await (await request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
});

function track(page: Page) {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', (response) => {
    if (response.status() >= 400)
      errors.push(`${response.status()} ${response.url()}`);
  });
  return errors;
}

// Leave the app before switching roles: a request it still sends would be
// answered for the new role, such as a 403 for an administrator page.
async function view(page: Page, role: 'admin' | 'user') {
  await page.goto('about:blank');
  await page.request.post('/__mock/viewer', { data: { role } });
}

const card = (page: Page, id: string) =>
  page.locator(`article[data-layout-key="library:${id}"]`);

async function cardMenu(page: Page, id: string, title: string) {
  await card(page, id).hover();
  await card(page, id)
    .getByRole('button', { name: `More actions for ${title}`, exact: true })
    .click();
  return page.getByRole('menu');
}

test('posters and title pages show Radarr and Sonarr status in each role', async ({
  page,
}, info) => {
  const errors = track(page);
  await page.goto('/#library/movies');
  await expect(card(page, 'tmdb:35').locator('[data-title-chip]')).toHaveText(
    '42%',
  );
  await expect(card(page, 'tmdb:38').locator('[data-title-chip]')).toHaveText(
    'Queued',
  );
  await expect(card(page, 'movie-22').locator('[data-title-chip]')).toHaveText(
    'Missing',
  );
  await expect(card(page, 'movie-23').locator('[data-title-chip]')).toHaveText(
    'Not monitored',
  );
  await expect(card(page, 'movie-0').locator('[data-title-chip]')).toHaveCount(
    0,
  );
  await card(page, 'tmdb:35').locator('.card-primary').click();
  await expect(
    page.getByRole('heading', { level: 1, name: 'The Quiet Harbor' }),
  ).toBeVisible();
  await expect(page.locator('[data-title-status="downloading"]')).toContainText(
    'Downloading 42%',
  );
  await page.getByRole('button', { name: 'Back to movies' }).click();

  await page.goto('/#library/shows/show-0');
  await expect(page.locator('[data-title-status]').first()).toHaveText(
    /8 of 9 episodes/,
  );
  await page.getByRole('tab', { name: 'Season 2' }).click();
  const missing = page.getByRole('listitem').filter({ hasText: 'Homeward' });
  await expect(missing.last()).toBeVisible();
  await expect(
    page.locator('ol [data-title-status="missing"]').first(),
  ).toHaveText('Missing');
  await expect(page.locator('ol [data-title-status="upcoming"]')).toHaveText(
    /^Coming /,
  );
  await page.screenshot({ path: info.outputPath('admin-series.png') });

  await view(page, 'user');
  await page.goto('/#library/movies');
  await expect(card(page, 'movie-22').locator('[data-title-chip]')).toHaveText(
    'Not available',
  );
  await expect(card(page, 'movie-23').locator('[data-title-chip]')).toHaveText(
    'Not available',
  );
  await page.goto('/#library/shows/show-0/season-1/episode-7');
  await expect(page.getByRole('tab', { name: 'Season 2' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.locator('ol [data-title-status="missing"]')).toHaveCount(0);
  await expect(page.locator('ol [data-title-status="unavailable"]')).toHaveText(
    'Not available',
  );
  await page.screenshot({ path: info.outputPath('user-series.png') });
  expect(errors).toEqual([]);
});

test('title menus keep administration actions behind the three dots', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/movies');
  let menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await expect(menu.getByText('Administration')).toBeVisible();
  await menu.getByRole('menuitemcheckbox', { name: 'Keep' }).click();
  await expect(
    page.getByText('Kept: automatic cleanup skips it.'),
  ).toBeVisible();
  menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await expect(
    menu.getByRole('menuitemcheckbox', { name: 'Keep' }),
  ).toHaveAttribute('aria-checked', 'true');
  await expect(
    menu.getByRole('menuitemcheckbox', { name: /^Monitored/ }),
  ).toHaveAttribute('aria-checked', 'true');
  // Kept files can't be deleted until Keep is turned off.
  await menu.getByRole('menuitem', { name: 'Delete files…' }).click();
  let dialog = page.getByRole('dialog', { name: /Delete files/ });
  await expect(dialog).toContainText('Keep is on. Turn it off first.');
  await expect(
    dialog.getByRole('button', { name: 'Delete files', exact: true }),
  ).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await menu.getByRole('menuitemcheckbox', { name: 'Keep' }).click();
  await expect(
    page.getByText('Automatic cleanup may remove it again.'),
  ).toBeVisible();
  menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await menu.getByRole('menuitem', { name: 'Delete files…' }).click();
  dialog = page.getByRole('dialog', { name: /Delete files/ });
  await expect(dialog.getByText('from disk', { exact: false })).toBeVisible();
  await dialog
    .getByRole('button', { name: 'Delete files', exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  // Deleting also unmonitors the title, so Radarr will not fetch it again.
  await expect(card(page, 'movie-1').locator('[data-title-chip]')).toHaveText(
    'Not monitored',
  );
  menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await expect(
    menu.getByRole('menuitemcheckbox', { name: /^Monitored/ }),
  ).toHaveAttribute('aria-checked', 'false');
  await page.keyboard.press('Escape');

  await view(page, 'user');
  await page.goto('/#library/movies');
  menu = await cardMenu(page, 'movie-0', 'Beyond the Horizon');
  await expect(
    menu.getByRole('menuitem', { name: 'Mark watched' }),
  ).toBeVisible();
  await expect(menu.getByText('Administration')).toHaveCount(0);
  await expect(menu.getByRole('menuitem', { name: 'Fix match…' })).toHaveCount(
    0,
  );
  await page.keyboard.press('Escape');
  await page.goto('/#library/movies/movie-0');
  await expect(
    page.getByRole('heading', { level: 1, name: 'Beyond the Horizon' }),
  ).toBeVisible();
  await page
    .getByRole('button', {
      name: 'More actions for Beyond the Horizon',
      exact: true,
    })
    .click();
  await expect(page.getByRole('menu').getByText('Administration')).toHaveCount(
    0,
  );
  expect(errors).toEqual([]);
});

test('a whole series or season joins Watch Later and leaves it once watched', async ({
  page,
}) => {
  const errors = track(page);
  await view(page, 'user');
  await page.goto('/#library/shows/show-1');
  const later = page.getByRole('button', { name: 'Watch Later', exact: true });
  await later.click();
  await expect(later).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('button', { name: 'More actions for Season 1' }).click();
  await page.getByRole('menuitem', { name: 'Add to Watch Later' }).click();
  await expect(page.getByText('Added to Watch Later.')).toBeVisible();
  await page.goto('/#home');
  await expect(
    page.locator('article[data-layout-key="home-Watch later:show-1"]'),
  ).toBeVisible();
  await expect(
    page.locator('article[data-layout-key="home-Watch later:season-2"]'),
  ).toBeVisible();
  for (let i = 8; i < 16; i++)
    expect(
      (
        await page.request.put(`/api/v1/catalog/episode-${i}/state`, {
          data: { watched: true },
        })
      ).ok(),
    ).toBe(true);
  for (const id of ['show-1', 'season-2'])
    expect(
      (await (await page.request.get(`/api/v1/catalog/${id}/state`)).json())
        .watch_later,
    ).toBe(false);
  await page.goto('/#library/shows/show-1');
  await expect(
    page.getByRole('button', { name: 'Watch Later', exact: true }),
  ).toHaveAttribute('aria-pressed', 'false');
  expect(errors).toEqual([]);
});

test('requests open their title and keep secondary actions in a menu', async ({
  page,
}) => {
  const errors = track(page);
  const history: string[] = [];
  page.on('request', (request) => {
    if (request.url().includes('/acquisition/requests'))
      history.push(request.url());
  });
  await page.goto('/#requests');
  await expect(
    page.getByRole('heading', { name: 'Media requests', exact: true }),
  ).toBeVisible();
  const pending = page
    .locator('article')
    .filter({ hasText: 'Pending approval' })
    .first();
  await expect(pending.getByRole('button', { name: 'Approve' })).toBeVisible();
  await pending.getByRole('button', { name: /^More actions for / }).click();
  await expect(
    page.getByRole('menuitem', { name: 'Cancel request' }),
  ).toBeVisible();
  await page.keyboard.press('Escape');
  await pending.locator('button').first().click();
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Back to discover', exact: true }),
  ).toBeVisible();

  await view(page, 'user');
  await page.goto('/#requests');
  const own = page.locator('article').filter({ hasText: 'Pending approval' });
  await expect(own.first()).toBeVisible();
  await expect(own.getByRole('button', { name: 'Approve' })).toHaveCount(0);
  await own
    .first()
    .getByRole('button', { name: /^More actions for / })
    .click();
  await expect(
    page.getByRole('menuitem', { name: 'Cancel request' }),
  ).toBeVisible();
  // The old request history list is gone, along with its requests.
  expect(history).toEqual([]);
  expect(errors).toEqual([]);
});

test('libraries scan without manual folder settings', async ({ page }) => {
  const errors = track(page);
  await page.goto('/#library/shows');
  await expect(card(page, 'show-0')).toBeVisible();
  await page.getByRole('button', { name: 'Shows library actions' }).click();
  await expect(
    page.getByRole('menuitem', { name: 'Library folders' }),
  ).toHaveCount(0);
  await page.getByRole('menuitem', { name: 'Scan library' }).click();
  await expect(page.getByText('Scanning 1 library folder.')).toBeVisible();
  await page.goto('/#settings/server');
  await expect(
    page.getByRole('heading', { name: 'Display defaults', exact: true }),
  ).toBeVisible();
  await expect(page.locator('#server-library')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Add folder' })).toHaveCount(0);

  await view(page, 'user');
  await page.goto('/#library/shows');
  await expect(
    page.getByRole('button', { name: 'Shows library actions' }),
  ).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('returning from a title restores where the library grid was scrolled', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/movies');
  const target = card(page, 'movie-19');
  await target.scrollIntoViewIfNeeded();
  const scroller = page.locator('.workspace-scroll');
  const before = await scroller.evaluate((element) => element.scrollTop);
  expect(before).toBeGreaterThan(0);
  await target.locator('.card-primary').click();
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await page
    .getByRole('button', { name: 'Back to movies', exact: true })
    .click();
  await expect(target).toBeVisible();
  await expect
    .poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBe(before);
  expect(errors).toEqual([]);
});

test('Home shelves keep every tile landscape without cropping posters', async ({
  page,
}, info) => {
  const errors = track(page);
  for (const [id, flag] of [
    ['movie-2', 'watch_later'],
    ['movie-1', 'watch_later'],
    ['album-3', 'favorite'],
    ['movie-1', 'favorite'],
  ])
    expect(
      (
        await page.request.put(`/api/v1/catalog/${id}/state`, {
          data: { [flag]: true },
        })
      ).ok(),
    ).toBe(true);
  await page.goto('/#home');
  const later = (id: string) =>
    page.locator(`article[data-layout-key="home-Watch later:${id}"]`);
  // Radarr's backdrop fills the tile; a title without one shows its whole poster.
  await later('movie-1').scrollIntoViewIfNeeded();
  await expect(later('movie-1').locator('img[data-art]')).toHaveAttribute(
    'data-art',
    'backdrop',
  );
  await later('movie-2').scrollIntoViewIfNeeded();
  await expect(later('movie-2').locator('img[data-art]')).toBeInViewport();
  await expect(later('movie-2').locator('img[data-art]')).toHaveAttribute(
    'data-art',
    'contained',
  );
  await expect(later('movie-2').locator('img[data-art-backdrop]')).toHaveCount(
    1,
  );
  const favorite = (id: string) =>
    page.locator(`article[data-layout-key="home-Favorites:${id}"] .tile-art`);
  await favorite('album-3').scrollIntoViewIfNeeded();
  const [album, movie] = await Promise.all(
    ['album-3', 'movie-1'].map((id) => favorite(id).boundingBox()),
  );
  expect(album!.height).toBeCloseTo(movie!.height, 0);
  expect(album!.width / album!.height).toBeCloseTo(16 / 9, 1);
  await page.screenshot({ path: info.outputPath('home-shelves.png') });
  expect(errors).toEqual([]);
});

test('a series only Sonarr knows lists its episodes from Discover and the Shows grid', async ({
  page,
}) => {
  const errors = track(page);
  for (const route of ['/#discover/tv/46', '/#library/shows/tmdb%3A46']) {
    await page.goto(route);
    await expect(
      page.getByRole('heading', { level: 1, name: 'Ember Coast' }),
    ).toBeVisible();
    await expect(page.getByRole('tab', { name: 'Season 1' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(page.getByRole('tabpanel').getByRole('listitem')).toHaveCount(
      6,
    );
  }
  expect(errors).toEqual([]);
});

test('status changes reach open grids and title pages without reloading', async ({
  page,
}) => {
  const errors = track(page);
  const advance = (progress: number) =>
    page.request.post('/__mock/titles', { data: { tmdb: 35, progress } });
  await page.goto('/#library/movies');
  const chip = card(page, 'tmdb:35').locator('[data-title-chip]');
  await expect(chip).toHaveText('42%');
  await advance(0.8);
  await expect(chip).toHaveText('80%');
  await chip.click();
  await expect(page.locator('[data-title-status="downloading"]')).toContainText(
    'Downloading 80%',
  );
  await advance(0.9);
  await expect(page.locator('[data-title-status="downloading"]')).toContainText(
    'Downloading 90%',
  );

  // A refresh keeps the season the reader picked over the one the link chose.
  await page.goto('/#library/shows/show-0/season-1/episode-7');
  await expect(page.getByRole('tab', { name: 'Season 2' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await page.getByRole('tab', { name: 'Season 1' }).click();
  await advance(0.95);
  await expect(page.getByRole('tab', { name: 'Season 1' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  expect(errors).toEqual([]);
});

test('Keep on a series covers its seasons and episodes and blocks their deletion', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/shows/show-2');
  await expect(
    page.getByRole('heading', { level: 1, name: 'The Last Signal' }),
  ).toBeVisible();
  await page
    .getByRole('button', {
      name: 'More actions for The Last Signal',
      exact: true,
    })
    .click();
  await page.getByRole('menuitemcheckbox', { name: 'Keep' }).click();
  await expect(
    page.getByText('Kept: automatic cleanup skips it.'),
  ).toBeVisible();

  await page
    .getByRole('button', { name: 'More actions for Season 1', exact: true })
    .click();
  const keep = page.getByRole('menuitemcheckbox', { name: /^Keep/ });
  await expect(keep).toHaveAttribute('aria-checked', 'true');
  await expect(keep).toHaveAttribute('aria-disabled', 'true');
  await expect(keep).toContainText('with the series');
  await expect(
    page.getByRole('menuitemcheckbox', { name: /^Monitored/ }),
  ).toHaveAttribute('aria-disabled', 'true');
  await page.getByRole('menuitem', { name: 'Delete season files…' }).click();
  const dialog = page.getByRole('dialog', { name: /Delete season files/ });
  await expect(dialog).toContainText('Keep is on for the whole series');
  await expect(
    dialog.getByRole('button', { name: 'Delete season files', exact: true }),
  ).toBeDisabled();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(errors).toEqual([]);
});
