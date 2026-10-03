import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

test('YouTube watchlist deep links reveal the selected list and Back restores it', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  await page.route('**/api/v1/online/youtube', (route) =>
    route.fulfill({
      json: {
        configured: true,
        account: { status: 'connected', display_name: 'Viewer', updated_at: 0 },
        sync: {},
      },
    }),
  );
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({
      json: [1, 2].map((id) => ({
        id,
        name: id === 1 ? 'Watch Later' : 'Study',
        isDefault: id === 1,
        autoDownload: false,
        autoRemoveWatched: false,
        sortMode: 'manual',
        sortDirection: 'desc',
        createdAt: '',
        updatedAt: '',
        items: [],
      })),
    }),
  );
  await page.route('**/api/v1/online/youtube/browse', (route) =>
    route.fulfill({
      json: {
        items: [],
        page: 0,
        pageSize: 60,
        hasMore: false,
        channels: [],
        counts: {
          all: 0,
          unwatched: 0,
          inProgress: 0,
          watched: 0,
          shorts: 0,
          live: 0,
          liveReplays: 0,
          upcoming: 0,
          subscribedChannelCount: 1,
        },
      },
    }),
  );
  await page.goto('/#online/youtube/watchlists/2');
  const study = page
    .locator('[data-youtube-watchlist-frame]')
    .getByRole('button', { name: /^Study/ });
  await expect(study).toBeVisible();
  await page.reload();
  await expect(study).toBeVisible();
  await study.click();
  await page
    .getByRole('dialog', { name: 'Watchlists', exact: true })
    .getByRole('button', { name: /^Watch Later/ })
    .click();
  await expect(page).toHaveURL(/#online\/youtube\/watchlists\/1$/);
  await page.goBack();
  await expect(study).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('watchlist-route', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

for (const role of ['admin', 'user'] as const) {
  test(`settings deep links respect ${role} access and survive reload`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { role });
    await page.goto('/#admin/server');
    await expect(
      page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', {
          name: role === 'admin' ? 'Server' : 'Account',
          exact: true,
        }),
    ).toHaveAttribute('aria-current', 'page');
    await expect(page).toHaveURL(
      role === 'admin' ? /#admin\/server$/ : /#settings\/account$/,
    );
    await page.reload();
    await expect(
      page.getByRole('heading', {
        name: role === 'admin' ? 'Server' : 'Account',
        exact: true,
      }),
    ).toBeVisible();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
    await testInfo.attach('settings-route', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  });
}

test('navigation links support new tabs, legacy URLs and Back/Forward across lazy pages', async ({
  page,
  context,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  await page.goto('/?section=Movies');
  await expect(page).toHaveURL(/#library\/movies$/);
  await expect(page.locator('.page-header h1')).toHaveText('Movies');
  const settings = page.getByRole('link', { name: 'Settings', exact: true });
  const popup = context.waitForEvent('page');
  await settings.click({ modifiers: ['Control'] });
  const tab = await popup;
  await expect.poll(() => tab.url()).toMatch(/#settings\/account$/);
  await tab.close();
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page.locator('.page-header h1')).toHaveText('Home');
  await page.goBack();
  await expect(page.locator('.page-header h1')).toHaveText('Movies');
  await page.goForward();
  await expect(page.locator('.page-header h1')).toHaveText('Home');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('history-route', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('catalog detail and search URLs survive reload and history restores the selection', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  await page.route('**/api/v1/catalog/movie-**', (route) => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1)!;
    return route.fulfill({
      json:
        id === 'state'
          ? { favorite: false, watched: false, watch_later: false }
          : {
              id,
              kind: 'movie',
              title: `Fixture movie ${id.split('-')[1]}`,
              available: true,
              metadata: {},
              files: [],
            },
    });
  });
  await page.goto('/#library/movies/movie-1?q=Fixture');
  const detail = page.getByRole('heading', {
    name: 'Fixture movie 1',
    exact: true,
    level: 2,
  });
  await expect(detail).toBeVisible();
  await expect(
    page.getByRole('textbox', { name: 'Search movies' }),
  ).toHaveValue('Fixture');
  await page.reload();
  await expect(detail).toBeVisible();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page).toHaveURL(/#library\/movies\?q=Fixture$/);
  await expect(detail).toHaveCount(0);
  await page.goBack();
  await expect(detail).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('catalog-route', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('catalog hierarchy URLs retain album context when closing a track and moving up', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Music' });
  await page.route('**/api/v1/playlists', (route) =>
    route.fulfill({ json: { items: [] } }),
  );
  const nodes = {
    artist: {
      id: 'artist',
      title: 'Artist',
      kind: 'artist',
      available: true,
      metadata: {},
    },
    album: {
      id: 'album',
      title: 'Album',
      kind: 'album',
      available: true,
      metadata: {},
    },
    track: {
      id: 'track',
      title: 'Track',
      kind: 'track',
      available: true,
      metadata: {},
      files: [],
    },
  };
  await page.route('**/api/v1/catalog**', (route) => {
    const url = new URL(route.request().url());
    const id = url.pathname.split('/').at(-1)!;
    if (id === 'catalog')
      return route.fulfill({
        json: {
          items: [
            nodes[
              url.searchParams.get('parent') === 'artist'
                ? 'album'
                : url.searchParams.get('parent') === 'album'
                  ? 'track'
                  : 'artist'
            ],
          ],
        },
      });
    return route.fulfill({
      json:
        id === 'state'
          ? { watched: false, favorite: false, watch_later: false }
          : nodes[id as keyof typeof nodes],
    });
  });
  await page.goto('/#library/music');
  await page
    .getByRole('button', { name: 'Artist artist', exact: true })
    .click();
  await page.getByRole('button', { name: 'Album album', exact: true }).click();
  await page
    .getByRole('button', { name: 'Details Track', exact: true })
    .click();
  await expect(page).toHaveURL(/#library\/music\/artist\/album\/track$/);
  await page.reload();
  await expect(
    page.getByRole('heading', { name: 'Track', exact: true, level: 2 }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page).toHaveURL(/#library\/music\/artist\/album$/);
  await expect(
    page.getByRole('button', { name: 'Details Track', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Album', exact: true }).click();
  await expect(page).toHaveURL(/#library\/music\/artist$/);
  await expect(
    page.getByRole('button', { name: 'Album album', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Artist', exact: true }).click();
  await expect(page).toHaveURL(/#library\/music$/);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('catalog-hierarchy', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('playlist drafts survive leaving the editor and can be discarded explicitly', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Playlists' });
  await page.route('**/api/v1/playlists**', (route) =>
    route.fulfill({ json: { items: [] } }),
  );
  await page.goto('/#playlists/new');
  const name = page.getByLabel('Playlist name', { exact: true });
  await name.fill('Unfinished playlist');
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.locator('.page-header h1')).toHaveText('Movies');
  await page.goBack();
  await expect(name).toHaveValue('Unfinished playlist');
  await expect(
    page.getByText('Unsaved playlist draft', { exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Discard changes', exact: true })
    .click();
  await expect(name).toHaveValue('');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('retained-draft', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('saving a new playlist replaces its creation URL and clears the new draft', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Playlists' });
  let playlist: Record<string, unknown> | undefined;
  await page.route('**/api/v1/playlists**', (route) => {
    if (route.request().method() === 'POST') {
      playlist = {
        ...route.request().postDataJSON(),
        id: 'created',
        owner_id: 'layout-fixture',
        owner: 'Viewer',
        count: 0,
        revision: 1,
        items: [],
      };
      return route.fulfill({ json: { id: 'created' } });
    }
    return route.fulfill({
      json: route.request().url().endsWith('/playlists')
        ? { items: playlist ? [playlist] : [] }
        : playlist,
    });
  });
  await page.goto('/#playlists/new');
  const name = page.getByLabel('Playlist name', { exact: true });
  await name.fill('Created playlist');
  await page
    .getByRole('button', { name: 'Save playlist', exact: true })
    .click();
  await expect(page).toHaveURL(/#playlists\/created$/);
  await expect(name).toHaveValue('Created playlist');
  await page.getByRole('button', { name: 'New playlist', exact: true }).click();
  await expect(name).toHaveValue('');
  await page.goBack();
  await expect(name).toHaveValue('Created playlist');
  await page.reload();
  await expect(name).toHaveValue('Created playlist');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('created-playlist-route', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('service selection is addressable and restored by browser history', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.goto('/#admin/services/radarr');
  const radarr = page.getByRole('link', { name: 'Radarr', exact: true });
  await expect(radarr).toHaveAttribute('aria-current', 'true');
  await page.getByRole('link', { name: 'Seerr', exact: true }).click();
  await expect(page).toHaveURL(/#admin\/services\/seerr$/);
  await page.goBack();
  await expect(radarr).toHaveAttribute('aria-current', 'true');
  await page.reload();
  await expect(radarr).toHaveAttribute('aria-current', 'true');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
  await testInfo.attach('service-route', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});
