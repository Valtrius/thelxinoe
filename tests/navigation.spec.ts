import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { responseGate } from './helpers/response-gate';

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

for (const section of ['Discover', 'Movies'] as const) {
  test(`${section} search keeps keyboard focus and caret across query updates`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section });
    await page.route('**/api/v1/seerr/**', (route) =>
      route.fulfill({
        json: new URL(route.request().url()).pathname.endsWith('/status')
          ? { configured: true, ready: true }
          : { results: [], page: 1, totalPages: 1, totalResults: 0 },
      }),
    );
    await page.goto('/');
    const input = page.getByRole('textbox', {
      name:
        section === 'Discover' ? 'Search movies and series' : 'Search movies',
      exact: true,
    });
    await input.click();
    for (const value of ['m', 'o', 'v', 'i', 'e']) {
      await page.keyboard.insertText(value);
      // Await the published URL, including the library's debounce, between keys.
      const term = await input.inputValue();
      await expect(page).toHaveURL(new RegExp(`q=${term}$`));
      await expect(input).toBeFocused();
      expect(
        await input.evaluate(
          (element: HTMLInputElement) => element.selectionStart,
        ),
      ).toBe(term.length);
    }
    await input.press('ArrowLeft');
    await page.keyboard.insertText('X');
    await expect(page).toHaveURL(/q=moviXe$/);
    await expect(input).toBeFocused();
    expect(
      await input.evaluate(
        (element: HTMLInputElement) => element.selectionStart,
      ),
    ).toBe(5);
    await testInfo.attach('search-focus', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('library filters retain workspace scroll when the results still fit it', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  await page.route('**/api/v1/catalog?**', (route) =>
    route.fulfill({
      json: {
        items: Array.from({ length: 60 }, (_, index) => ({
          id: `movie-${index}`,
          kind: 'movie',
          title: `Movie ${index}`,
          available: true,
          metadata: {},
        })),
      },
    }),
  );
  await page.route('**/api/v1/catalog/collections', (route) =>
    route.fulfill({ json: { items: [{ id: 1, name: 'Series', count: 60 }] } }),
  );
  await page.goto('/');
  await expect(
    page.getByRole('combobox', { name: 'Collection', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Details Movie 59', exact: true }),
  ).toBeAttached();
  const before = await page.locator('.workspace-scroll').evaluate((element) => {
    element.scrollTop = 120;
    return element.scrollTop;
  });
  expect(before).toBe(120);
  await page
    .getByRole('combobox', { name: 'Collection', exact: true })
    .evaluate((element: HTMLSelectElement) => {
      element.value = '1';
      element.dispatchEvent(new Event('change', { bubbles: true }));
    });
  await expect(page).toHaveURL(/collection=1/);
  await expect
    .poll(() =>
      page
        .locator('.workspace-scroll')
        .evaluate((element) => element.scrollTop),
    )
    .toBe(before);
  await testInfo.attach('filter-scroll', {
    body: JSON.stringify({ scroll: before, url: page.url() }),
    contentType: 'application/json',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const decision of [
  'unchanged',
  'saved',
  'replace',
  'racing replace',
] as const) {
  test(`retained playlist draft keeps its original revision with ${decision} changes`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, { section: 'Playlists' });
    let playlist = {
      id: 'shared',
      name: 'Shared playlist',
      description: '',
      owner_id: 'layout-fixture',
      owner: 'Viewer',
      favorite: false,
      count: 1,
      revision: 1,
      items: [
        {
          id: 'original',
          kind: 'track',
          title: 'Original track',
          available: true,
        },
      ],
    };
    const writes: { revision: number; name: string; items: string[] }[] = [];
    const conflictReload = responseGate();
    let conflictReloadStarted = false;
    await page.route('**/api/v1/playlists**', async (route) => {
      const path = new URL(route.request().url()).pathname;
      if (route.request().method() === 'PUT') {
        const body = route.request().postDataJSON();
        writes.push(body);
        if (decision === 'racing replace' && writes.length === 1)
          playlist = {
            ...playlist,
            revision: 3,
            count: 3,
            items: [
              ...playlist.items,
              {
                id: 'later',
                kind: 'track',
                title: 'Later track',
                available: true,
              },
            ],
          };
        if (body.revision !== playlist.revision)
          return route.fulfill({
            status: 409,
            json: {
              error: {
                code: 'conflict',
                message: 'This playlist changed; reload before editing',
              },
            },
          });
        playlist = {
          ...playlist,
          name: body.name,
          revision: playlist.revision + 1,
          items: body.items.map((id: string) => ({
            id,
            kind: 'track',
            title:
              id === 'remote'
                ? 'Remote track'
                : id === 'later'
                  ? 'Later track'
                  : 'Original track',
            available: true,
          })),
        };
        return route.fulfill({ json: { id: playlist.id } });
      }
      if (
        decision === 'racing replace' &&
        writes.length === 1 &&
        path.endsWith('/shared')
      ) {
        conflictReloadStarted = true;
        await conflictReload.promise;
      }
      return route.fulfill({
        json: path.endsWith('/playlists') ? { items: [playlist] } : playlist,
      });
    });
    await page.goto('/#playlists/shared');
    const name = page.getByLabel('Playlist name', { exact: true });
    await name.fill('Local draft');
    await page.getByRole('link', { name: 'Movies', exact: true }).click();
    if (decision !== 'unchanged')
      playlist = {
        ...playlist,
        revision: 2,
        count: 2,
        items: [
          ...playlist.items,
          {
            id: 'remote',
            kind: 'track',
            title: 'Remote track',
            available: true,
          },
        ],
      };
    await page.goBack();
    await expect(name).toHaveValue('Local draft');
    if (decision !== 'unchanged') {
      await expect(page.getByRole('alert')).toContainText('changed elsewhere');
      await expect(
        page.getByRole('button', { name: 'Save playlist', exact: true }),
      ).toBeDisabled();
      await page.getByRole('link', { name: 'Movies', exact: true }).click();
      await page.goBack();
      await expect(page.getByRole('alert')).toContainText('changed elsewhere');
      expect(writes).toEqual([]);
      await page.getByText('Saved playlist', { exact: true }).click();
      await expect(page.getByRole('alert')).toContainText('Remote track');
      await page.screenshot({
        path: testInfo.outputPath('playlist-conflict.png'),
      });
      if (decision === 'saved') {
        await page
          .getByRole('button', { name: 'Use saved playlist', exact: true })
          .click();
        await expect(name).toHaveValue('Shared playlist');
        await expect(
          page.getByText('2. Remote track', { exact: true }),
        ).toBeVisible();
        await name.fill('Merged name');
        await page
          .getByRole('button', { name: 'Save playlist', exact: true })
          .click();
      } else {
        await page
          .getByRole('button', { name: 'Replace with my draft', exact: true })
          .click();
        if (decision === 'racing replace') {
          await expect.poll(() => conflictReloadStarted).toBe(true);
          await page.screenshot({
            path: testInfo.outputPath('playlist-conflict-refresh-pending.png'),
          });
          await expect(page.getByRole('alert')).toHaveCount(1);
          await expect(name).toHaveValue('Local draft');
          await expect(
            page.getByRole('button', {
              name: 'Replace with my draft',
              exact: true,
            }),
          ).toBeDisabled();
          conflictReload.release();
          await expect(page.getByRole('alert')).toContainText('Later track');
          await expect(name).toHaveValue('Local draft');
          await expect(
            page.getByRole('button', { name: 'Save playlist', exact: true }),
          ).toBeDisabled();
          expect(writes).toHaveLength(1);
          expect(writes[0]).toMatchObject({ revision: 2, items: ['original'] });
          await page
            .getByRole('button', { name: 'Use saved playlist', exact: true })
            .click();
          await expect(
            page.getByText('3. Later track', { exact: true }),
          ).toBeVisible();
          await name.fill('Merged name');
          await page
            .getByRole('button', { name: 'Save playlist', exact: true })
            .click();
        }
      }
    } else
      await page
        .getByRole('button', { name: 'Save playlist', exact: true })
        .click();
    await expect
      .poll(() => writes.length)
      .toBe(decision === 'racing replace' ? 2 : 1);
    const saved = writes.at(-1)!;
    expect(saved.revision).toBe(
      decision === 'unchanged' ? 1 : decision === 'racing replace' ? 3 : 2,
    );
    expect(saved.items).toEqual(
      decision === 'saved'
        ? ['original', 'remote']
        : decision === 'racing replace'
          ? ['original', 'remote', 'later']
          : ['original'],
    );
    await expect(name).toHaveValue(
      decision === 'saved' || decision === 'racing replace'
        ? 'Merged name'
        : 'Local draft',
    );
    await expect(
      page.getByText('Unsaved playlist draft', { exact: true }),
    ).toHaveCount(0);
    await testInfo.attach('playlist-revision-decision', {
      body: JSON.stringify({ decision, writes, playlist }),
      contentType: 'application/json',
    });
    await page.screenshot({
      path: testInfo.outputPath('playlist-conflict-resolved.png'),
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

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
