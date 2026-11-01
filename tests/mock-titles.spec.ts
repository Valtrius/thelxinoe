import { expect, test, type Locator, type Page } from '@playwright/test';
import { hold } from './helpers/touch';

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
  // Only what will not come is greyed out, whether Radarr has it alone or not.
  const art = (id: string) => card(page, id).locator('img[data-art]');
  await expect(art('movie-23')).toHaveClass(/grayscale/);
  await expect(art('tmdb:37')).not.toHaveClass(/grayscale/);
  await page.goto('/#library/shows');
  await expect(
    page.locator('[data-title-chip="available"]').first(),
  ).toHaveText(/^\d+\/\d+$/);
  await page.goto('/#library/movies');
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

test('unmatched movies offer a match without a failing refresh action', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/movies');
  let menu = await cardMenu(page, 'movie-0', 'Beyond the Horizon');
  await expect(
    menu.getByRole('menuitem', { name: 'Refresh metadata' }),
  ).toHaveCount(0);
  await menu.getByRole('menuitem', { name: 'Fix match…' }).click();
  const dialog = page.getByRole('dialog', {
    name: 'Fix match · Beyond the Horizon',
  });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole('button', { name: 'Refresh metadata' }),
  ).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Search media service' }).click();
  await expect(
    dialog.getByRole('button', { name: 'Use this match' }).first(),
  ).toBeVisible();
  const matched = page.waitForRequest(
    (request) =>
      request.method() === 'POST' &&
      request.url().endsWith('/catalog/movie-0/match'),
  );
  await dialog.getByRole('button', { name: 'Use this match' }).first().click();
  expect((await matched).postDataJSON()).toEqual({
    service_id: 'manager-radarr',
    service_generation: 'preview',
    external_id: 'movie-0',
  });
  await expect(dialog.getByRole('status')).toHaveText('Match queued.');
  await page.keyboard.press('Escape');
  menu = await cardMenu(page, 'movie-1', 'Nightfall');
  const refreshed = page.waitForRequest(
    (request) =>
      request.method() === 'POST' &&
      request.url().endsWith('/catalog/movie-1/refresh'),
  );
  await menu.getByRole('menuitem', { name: 'Refresh metadata' }).click();
  await refreshed;
  expect(errors).toEqual([]);
});

test('movies can leave Radarr and its lists, with the choices remembered', async ({
  page,
}, info) => {
  const errors = track(page);
  await page.goto('/#library/movies');
  let menu = await cardMenu(page, 'movie-1', 'Nightfall');
  await menu.getByRole('menuitem', { name: 'Delete files…' }).click();
  let dialog = page.getByRole('dialog', { name: /Delete files/ });
  const remove = () =>
    dialog.getByRole('checkbox', { name: 'Remove from Radarr', exact: true });
  const exclude = () =>
    dialog.getByRole('checkbox', {
      name: "Add to Radarr's exclusion list",
      exact: true,
    });
  await expect(remove()).not.toBeChecked();
  await expect(exclude()).not.toBeChecked();
  await remove().check();
  await exclude().check();
  await dialog.screenshot({ path: info.outputPath('delete-movie.png') });
  const executed = page.waitForRequest(/\/operations\/[^/]+\/execute$/);
  await dialog
    .getByRole('button', { name: 'Delete files', exact: true })
    .click();
  expect((await executed).postDataJSON()).toEqual({
    remove: true,
    exclude: true,
  });
  await expect(dialog).toHaveCount(0);
  // Without files or Radarr, the movie leaves the library.
  await expect(card(page, 'movie-1')).toHaveCount(0);

  await expect
    .poll(async () => {
      const saved = await (
        await page.request.get('/api/v1/me/appearance')
      ).json();
      return saved.provider_preferences['movie-deletion'];
    })
    .toBe(JSON.stringify({ remove: true, exclude: true }));
  await page.reload();
  menu = await cardMenu(page, 'movie-7', 'Parallel Worlds');
  await menu.getByRole('menuitem', { name: 'Delete files…' }).click();
  dialog = page.getByRole('dialog', { name: /Delete files/ });
  await expect(remove()).toBeChecked();
  await expect(exclude()).toBeChecked();
  await remove().uncheck();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();

  // An unmonitored movie without files can only be removed from Radarr.
  menu = await cardMenu(page, 'movie-23', 'First Light II');
  await expect(
    menu.getByRole('menuitem', { name: 'Delete files…' }),
  ).toHaveCount(0);
  await menu.getByRole('menuitem', { name: 'Remove from Radarr…' }).click();
  dialog = page.getByRole('dialog', { name: /Remove from Radarr/ });
  await expect(exclude()).toBeChecked();
  await dialog.screenshot({ path: info.outputPath('remove-movie.png') });
  let removed = page.waitForRequest(/\/admin\/movies\//);
  await dialog
    .getByRole('button', { name: 'Remove from Radarr', exact: true })
    .click();
  let request = await removed;
  expect(request.method()).toBe('DELETE');
  expect(new URL(request.url()).pathname).toBe('/api/v1/admin/movies/movie-23');
  expect(new URL(request.url()).searchParams.get('exclude')).toBe('true');
  await expect(dialog).toHaveCount(0);
  await expect(card(page, 'movie-23')).toHaveCount(0);

  // So can a movie only Radarr lists, which then leaves the library.
  menu = await cardMenu(page, 'tmdb:37', 'Signal Fire');
  await menu.getByRole('menuitem', { name: 'Remove from Radarr…' }).click();
  dialog = page.getByRole('dialog', { name: /Remove from Radarr/ });
  await exclude().uncheck();
  removed = page.waitForRequest(/\/admin\/movies\//);
  await dialog
    .getByRole('button', { name: 'Remove from Radarr', exact: true })
    .click();
  request = await removed;
  expect(decodeURIComponent(new URL(request.url()).pathname)).toBe(
    '/api/v1/admin/movies/tmdb:37',
  );
  expect(new URL(request.url()).searchParams.get('exclude')).toBe('false');
  await expect(card(page, 'tmdb:37')).toHaveCount(0);

  await view(page, 'user');
  await page.goto('/#library/movies');
  await expect(card(page, 'tmdb:38')).toBeVisible();
  await expect(
    card(page, 'tmdb:38').getByRole('button', { name: /^More actions/ }),
  ).toHaveCount(0);
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
  // Music requests follow, without approval for a regular user.
  const music = page.getByRole('region', { name: 'Music requests' });
  await expect(music).toContainText('Night Ferry');
  await expect(music.getByRole('button', { name: 'Approve' })).toHaveCount(0);
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

for (const touch of [false, true])
  test.describe(touch ? 'tap' : 'click', () => {
    test.use({ hasTouch: touch });

    test('library menus open on a press and pressing away only closes them', async ({
      page,
    }) => {
      const errors = track(page);
      await page.goto('/#library/movies');
      const first = card(page, 'movie-0');
      const below = card(page, 'movie-1').locator('.card-primary');
      const menu = page.getByRole('menu');
      const url = page.url();
      const press = (target: Locator) =>
        touch ? target.tap() : target.click();
      // Forced, as a user presses wherever the card shows, covered or not.
      const pressBelow = () =>
        touch ? below.tap({ force: true }) : below.click({ force: true });

      // Touch devices hide the card's buttons and open its menu on a long press.
      if (touch)
        await hold(page, first.locator('.card-primary'), () =>
          expect(
            menu.getByRole('menuitem', { name: 'Mark watched' }),
          ).toBeVisible(),
        );
      else {
        await first.hover();
        await first.getByRole('button', { name: /^More actions for / }).click();
      }
      await expect(menu).toBeVisible();
      expect(page.url()).toBe(url);
      await pressBelow();
      await expect(menu).toHaveCount(0);
      expect(page.url()).toBe(url);

      // A tap opens this menu on its pointerup; the tap's click must not close it.
      await press(
        page.getByRole('button', {
          name: 'Movies library actions',
          exact: true,
        }),
      );
      await expect(menu).toBeVisible();
      await pressBelow();
      await expect(menu).toHaveCount(0);
      expect(page.url()).toBe(url);

      await page
        .getByRole('button', { name: 'Filter movies', exact: true })
        .click();
      const filters = page.getByRole('dialog', { name: 'Filter movies' });
      await expect(filters).toBeVisible();
      await pressBelow();
      await expect(filters).toHaveCount(0);
      expect(page.url()).toBe(url);
      expect(errors).toEqual([]);
    });
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

test('libraries list every Sonarr series, sort and filter them, and Discover marks them', async ({
  page,
}) => {
  const errors = track(page);
  const tiles = page.locator('article.library-tile');
  await page.goto('/#library/shows');
  // A series watched long ago that Sonarr keeps without files.
  await expect(card(page, 'tmdb:47')).toContainText('Ended');
  await expect(tiles).toHaveCount(26);
  await page.getByRole('button', { name: 'Filter shows', exact: true }).click();
  const filters = page.getByRole('dialog', { name: 'Filter shows' });
  await filters.getByRole('button', { name: /^Not downloaded/ }).click();
  await expect(tiles).toHaveCount(1);
  await filters.getByRole('button', { name: /^Upcoming/ }).click();
  await expect(tiles).toHaveCount(2);
  await filters.getByRole('button', { name: /^Ended/ }).click();
  await expect(tiles).toHaveCount(1);
  await expect(card(page, 'tmdb:47')).toBeVisible();
  // Each user's view is kept for their next visit, once its save lands.
  await expect
    .poll(async () => {
      const saved = await (
        await page.request.get('/api/v1/me/appearance')
      ).json();
      return JSON.parse(
        saved.provider_preferences['library-shows-view'] ?? '{}',
      ).series;
    })
    .toBe('ended');
  await page.reload();
  await expect(tiles).toHaveCount(1);
  await page.getByRole('button', { name: 'Filter shows', exact: true }).click();
  for (const name of [/^Not downloaded/, /^Upcoming/, /^Ended/])
    await expect(filters.getByRole('button', { name })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  await filters.getByRole('button', { name: 'Clear', exact: true }).click();
  await expect(tiles).toHaveCount(26);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: 'Sort shows', exact: true }).click();
  const sorts = page.getByRole('dialog', { name: 'Sort shows' });
  await sorts
    .getByRole('button', { name: 'Year ascending', exact: true })
    .click();
  await expect(tiles.first()).toHaveAttribute(
    'data-layout-key',
    'library:tmdb:47',
  );
  // Cards show the value they are sorted by, as Sonarr's posters do.
  await sorts
    .getByRole('button', { name: 'Size on disk descending', exact: true })
    .click();
  await expect(tiles.first()).toHaveAttribute(
    'data-layout-key',
    'library:show-9',
  );
  await expect(tiles.first()).toContainText('13 GB');
  await expect(card(page, 'tmdb:47')).not.toContainText('GB');

  await page.goto('/#discover');
  const poster = (name: string) =>
    page.getByRole('button', { name: `View ${name}`, exact: true }).first();
  await expect(poster('Harbor Lights').locator('[data-in-library]')).toHaveText(
    'In library',
  );
  await expect(
    poster('Beyond the Horizon').locator('[data-in-library]'),
  ).toHaveCount(0);
  await page.goto('/#discover/tv/47');
  await expect(
    page.getByRole('heading', { level: 1, name: 'Harbor Lights' }),
  ).toBeVisible();
  await expect(
    page.getByRole('article').getByText('Ended', { exact: true }),
  ).toBeVisible();
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

test('phone episode rows put the overview and Sonarr actions under the still, across the row', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/shows/show-0');
  await page.getByRole('tab', { name: 'Season 2' }).click();
  const rows = page
    .getByRole('list', { name: 'Season 2 episodes' })
    .getByRole('listitem');
  const row = rows.first();
  const still = row.getByRole('button', { name: /^Play episode 1:/ });
  const overview = row.locator('p').filter({ hasText: 'A remote observatory' });
  const missing = rows.filter({
    has: page.locator('[data-title-status="missing"]'),
  });
  const search = missing.getByRole('button', { name: 'Search', exact: true });
  const menu = missing.getByRole('button', { name: /^More actions for / });
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(overview).toBeVisible();
    await expect(search).toBeVisible();
    const [rowBox, stillBox, overviewBox, searchBox, menuBox] =
      await Promise.all(
        [row, still, overview, search, menu].map(
          async (part) => (await part.boundingBox())!,
        ),
      );
    // The still keeps its inset from the Next up rail.
    expect(stillBox.x - rowBox.x).toBeGreaterThanOrEqual(8);
    if (width > 720) {
      expect(overviewBox.x).toBeGreaterThan(stillBox.x + stillBox.width);
      expect(searchBox.x).toBeCloseTo(overviewBox.x, 0);
    } else {
      expect(overviewBox.x).toBeCloseTo(stillBox.x, 0);
      expect(overviewBox.y).toBeGreaterThan(stillBox.y + stillBox.height);
      expect(overviewBox.width).toBeGreaterThan(rowBox.width - 20);
      expect(searchBox.x).toBeCloseTo(stillBox.x, 0);
      expect(searchBox.y).toBeGreaterThan(menuBox.y + menuBox.height);
    }
  }
  expect(errors).toEqual([]);
});

test('episode rows show stills, mark seasons watched and search Sonarr', async ({
  page,
}) => {
  const errors = track(page);
  await page.goto('/#library/shows/show-0');
  await page.getByRole('tab', { name: 'Season 2' }).click();
  const list = page.getByRole('list', { name: 'Season 2 episodes' });
  await expect(list.locator('img').first()).toHaveAttribute(
    'src',
    /\/mock-art\/video\//,
  );
  const missing = list
    .getByRole('listitem')
    .filter({ has: page.locator('[data-title-status="missing"]') });

  // Interactive search lists Sonarr's releases, rejected ones on request.
  await missing.getByRole('button', { name: 'Interactive search…' }).click();
  const dialog = page.getByRole('dialog', {
    name: /^Interactive search · S02E05/,
  });
  await expect(dialog.getByText('Best match')).toBeVisible();
  await expect(dialog).toContainText('1 rejected hidden');
  await dialog.getByText('Show rejected').click();
  await expect(
    dialog.getByText('720p HDTV is not wanted in profile HD-1080p'),
  ).toBeVisible();
  await dialog
    .getByRole('button', { name: /^Download: Show\.S02E05\.1080p/ })
    .click();
  await expect(page.getByText('Sent to the download client.')).toBeVisible();
  await expect(dialog).toHaveCount(0);

  await page.getByRole('button', { name: 'Search missing' }).click();
  await expect(
    page.getByText('Sonarr is searching for 1 missing episode.'),
  ).toBeVisible();
  await missing.getByRole('button', { name: /^More actions for / }).click();
  const menu = page.getByRole('menu');
  await expect(
    menu.getByRole('menuitem', { name: 'Automatic search' }),
  ).toBeVisible();
  await expect(
    menu.getByRole('menuitemcheckbox', { name: 'Monitored' }),
  ).toHaveAttribute('aria-checked', 'true');
  await expect(menu.getByText(/^Last searched /)).toBeVisible();
  await page.keyboard.press('Escape');

  await page.getByRole('button', { name: 'Mark season watched' }).click();
  await expect(
    page.getByRole('button', { name: 'Mark season unwatched' }),
  ).toBeVisible();
  await expect(
    list.getByRole('button', { name: /^Watched: / }).first(),
  ).toHaveAttribute('aria-pressed', 'true');

  // Members mark what they watched but see none of Sonarr's actions.
  await view(page, 'user');
  await page.goto('/#library/shows/show-0');
  await page.getByRole('tab', { name: 'Season 2' }).click();
  await expect(
    page.getByRole('button', { name: 'Search missing' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Interactive search…' }),
  ).toHaveCount(0);
  await page.getByRole('tab', { name: 'Season 1' }).click();
  await page
    .getByRole('list', { name: 'Season 1 episodes' })
    .getByRole('button', { name: 'More actions for The Signal', exact: true })
    .click();
  await page.getByRole('menuitem', { name: 'Mark watched up to here' }).click();
  await expect(
    page.getByText('Marked watched up to this episode.'),
  ).toBeVisible();
  await expect(page.getByRole('tab', { name: 'Season 1' })).toContainText(
    '3/4',
  );
  await page.getByRole('button', { name: 'Unwatched 1' }).click();
  await expect(
    page.getByRole('list', { name: 'Season 1 episodes' }).getByRole('listitem'),
  ).toHaveCount(1);
  expect(errors).toEqual([]);
});
