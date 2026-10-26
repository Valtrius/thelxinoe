import { expect, test, type Page } from '@playwright/test';
import { installServerEvents, installUiFixture } from './helpers/ui-fixture';
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
  await page
    .getByRole('button', { name: 'Filter movies', exact: true })
    .click();
  await expect(
    page.getByRole('combobox', { name: 'Collection', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', {
      name: 'More actions for Movie 59',
      exact: true,
    }),
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

async function routeYoutubeWatchlists(page: Page, names: string[]) {
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
      json: names.map((name, index) => ({
        id: index + 1,
        name,
        isDefault: index === 0,
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
}

test('YouTube watchlist deep links reveal the selected list and Back restores it', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  await routeYoutubeWatchlists(page, ['Watch Later', 'Study']);
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

// The manifest's share target opens the app with the share in its address.
function sharedAddress(share: Record<string, string>) {
  return `/?${new URLSearchParams(share)}`;
}

async function routeWatchlistAdditions(page: Page) {
  const additions: { watchlist: string; body: unknown }[] = [];
  await page.route('**/api/v1/online/youtube/watchlists/*/items', (route) => {
    additions.push({
      watchlist: route.request().url().split('/').at(-2)!,
      body: route.request().postDataJSON(),
    });
    return route.fulfill({
      json: {
        added: true,
        video: {
          videoId: 'xwdamnfS6IM',
          title: 'xwdamnfS6IM',
          metadataPending: true,
        },
        autoDownloadError: null,
      },
    });
  });
  return additions;
}

test('a video shared before signing in joins the only watchlist', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await routeYoutubeWatchlists(page, ['Watch Later']);
  const additions = await routeWatchlistAdditions(page);
  await page.goto(
    sharedAddress({
      share_text: 'Watch https://youtu.be/xwdamnfS6IM?si=shared',
    }),
  );
  await expect(page).not.toHaveURL(/share_text/);
  await page.getByLabel('Username', { exact: true }).fill('viewer');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('status').filter({ hasText: 'Added to Watch Later.' }),
  ).toBeVisible();
  expect(additions).toEqual([
    { watchlist: '1', body: { videoId: 'xwdamnfS6IM', manualPosition: 0 } },
  ]);
  await testInfo.attach('shared-video-saved', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('a shared video asks which watchlist to join when there are several', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  await routeYoutubeWatchlists(page, ['Watch Later', 'Study']);
  const additions = await routeWatchlistAdditions(page);
  await page.goto(
    sharedAddress({ share_url: 'https://www.youtube.com/watch?v=xwdamnfS6IM' }),
  );
  const dialog = page.getByRole('dialog', {
    name: 'Save to watchlist',
    exact: true,
  });
  await testInfo.attach('shared-video-choice', {
    body: await dialog.screenshot(),
    contentType: 'image/png',
  });
  await dialog.getByRole('button', { name: /^Study/ }).click();
  await expect(dialog).toHaveCount(0);
  await page
    .getByRole('status')
    .filter({ hasText: 'Added to Study.' })
    .getByRole('button', { name: 'Open', exact: true })
    .click();
  await expect(page).toHaveURL(/#online\/youtube\/watchlists\/2$/);
  await expect(
    page
      .locator('[data-youtube-watchlist-frame]')
      .getByRole('button', { name: /^Study/ }),
  ).toBeVisible();
  expect(additions).toEqual([
    { watchlist: '2', body: { videoId: 'xwdamnfS6IM', manualPosition: 0 } },
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('the server alone removes a watched video from an auto-remove watchlist', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'YouTube' });
  const events = await installServerEvents(page);
  await page.route('**/api/v1/online/youtube', (route) =>
    route.fulfill({
      json: {
        configured: true,
        account: { status: 'connected', display_name: 'Viewer', updated_at: 0 },
        sync: {},
      },
    }),
  );
  const video = {
    videoId: 'finished01',
    channelId: 'channel',
    channelName: 'Fixture channel',
    title: 'Finished video',
    publishedAt: '2026-09-21T00:00:00Z',
    durationSeconds: 60,
    broadcastState: 'none',
    isLive: false,
    isUpcoming: false,
    isLiveReplay: false,
    positionSeconds: 0,
    watchedPercentage: 0,
    isWatched: false,
  };
  let items = [{ video, addedAt: video.publishedAt, manualPosition: 0 }];
  await page.route('**/api/v1/online/youtube/watchlists', (route) =>
    route.fulfill({
      json: [
        {
          id: 1,
          name: 'Watch Later',
          isDefault: true,
          autoDownload: false,
          autoRemoveWatched: true,
          sortMode: 'manual',
          sortDirection: 'desc',
          createdAt: '',
          updatedAt: '',
          items,
        },
      ],
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
  await page.route(
    `**/api/v1/online/youtube/videos/${video.videoId}`,
    (route) => route.fulfill({ json: {} }),
  );
  await page.clock.install();
  await page.goto('/#online/youtube/watchlists/1');
  await expect.poll(events.connected).toBe(true);
  const row = page.locator(`[data-watchlist-video-id="${video.videoId}"]`);
  await row.hover();
  await row.getByRole('button', { name: 'Mark watched', exact: true }).click();
  // The watched video stays listed past the removal delay; the client never
  // deletes it itself.
  const unwatch = row.getByRole('button', {
    name: 'Mark unwatched',
    exact: true,
  });
  await expect(unwatch).toBeAttached();
  await page.clock.runFor(8000);
  await expect(unwatch).toBeAttached();
  items = [];
  events.send('youtube.changed');
  await expect(row).toHaveCount(0);
  expect(fixture.writes.filter(({ method }) => method === 'DELETE')).toEqual(
    [],
  );
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
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
}, testInfo) => {
  const fixture = await installUiFixture(page);
  await page.goto('/?section=Movies');
  await expect(page).toHaveURL(/#library\/movies$/);
  await expect(page.locator('.page-header h1')).toHaveText('Movies');
  const settings = page.getByRole('link', { name: 'Settings', exact: true });
  // A modified click belongs to the browser, which opens the link's own URL in
  // a new tab. Playwright sometimes never reports such a background tab, so the
  // test checks that the app leaves the click alone, then keeps the tab closed.
  await page.evaluate(() =>
    window.addEventListener(
      'click',
      (event) => {
        Object.assign(window, { modifiedClickHandled: event.defaultPrevented });
        event.preventDefault();
      },
      { once: true },
    ),
  );
  await settings.click({ modifiers: ['Control'] });
  expect(
    await page.evaluate(
      () =>
        (window as unknown as { modifiedClickHandled: boolean })
          .modifiedClickHandled,
    ),
  ).toBe(false);
  expect(
    await settings.evaluate((link) => (link as HTMLAnchorElement).href),
  ).toMatch(/#settings\/account$/);
  await expect(page).toHaveURL(/#library\/movies$/);
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
    level: 1,
  });
  await expect(detail).toBeVisible();
  await page.reload();
  await expect(detail).toBeVisible();
  await page
    .getByRole('button', { name: 'Back to movies', exact: true })
    .click();
  await expect(page).toHaveURL(/#library\/movies\?q=Fixture$/);
  await expect(detail).toHaveCount(0);
  await expect(
    page.getByRole('textbox', { name: 'Search movies' }),
  ).toHaveValue('Fixture');
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

for (const viewport of [
  { width: 1440, height: 700 },
  { width: 390, height: 844 },
]) {
  // Wide layouts scroll only the settings column and open each section at its
  // top. Compact layouts scroll the whole workspace and keep its position.
  const compact = viewport.width <= 720;
  test(`settings sections and service tabs keep their scroll at ${viewport.width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize(viewport);
    const fixture = await installUiFixture(page, { role: 'admin' });
    await page.goto('/');
    await expect(
      page.getByRole('heading', {
        name: 'Your display preferences',
        exact: true,
      }),
    ).toBeVisible();
    const workspace = page.locator('.workspace-scroll');
    const scroller = compact ? workspace : page.locator('.settings-scroll');
    const nav = page.getByRole('navigation', { name: 'Settings navigation' });
    const scroll = async () =>
      scroller.evaluate((element) => element.scrollTop);
    const position = 80;
    await scroller.evaluate((element, top) => {
      element.scrollTop = top;
    }, position);
    expect(await scroll()).toBe(position);
    for (const name of ['Devices', 'Account', 'Media services']) {
      const link = nav.getByRole('link', { name, exact: true });
      await link.click();
      await expect(page.locator('.settings-header > h1')).toHaveText(name);
      await expect(page.locator('[data-page-content]')).toHaveAttribute(
        'data-page-phase',
        'idle',
      );
      await expect.poll(scroll).toBe(compact ? position : 0);
      await expect(link).toBeFocused();
    }
    // On wide layouts the service tabs lead the settings column, so clicking
    // one brings the column to its top; history still keeps the position.
    const services = page.getByRole('navigation', { name: 'Select service' });
    for (const name of ['Radarr', 'Sonarr', 'NZBGet', 'Seerr']) {
      const link = services.getByRole('link', { name, exact: true });
      await link.click();
      await expect(link).toHaveAttribute('aria-current', 'true');
      await expect(page.locator('[data-page-content]')).toHaveAttribute(
        'data-page-phase',
        'idle',
      );
      await expect.poll(scroll).toBe(compact ? position : 0);
      await expect(link).toBeFocused();
    }
    if (!compact) {
      await scroller.evaluate((element, top) => {
        element.scrollTop = top;
      }, position);
      expect(await scroll()).toBe(position);
    }
    await page.goBack();
    await expect(page).toHaveURL(/#admin\/services\/nzbget$/);
    await expect(
      services.getByRole('link', { name: 'NZBGet', exact: true }),
    ).toHaveAttribute('aria-current', 'true');
    await expect(page.locator('[data-page-content]')).toHaveAttribute(
      'data-page-phase',
      'idle',
    );
    await expect.poll(scroll).toBe(position);
    await page.goForward();
    await expect(page).toHaveURL(/#admin\/services\/seerr$/);
    await expect(
      services.getByRole('link', { name: 'Seerr', exact: true }),
    ).toHaveAttribute('aria-current', 'true');
    await expect(page.locator('[data-page-content]')).toHaveAttribute(
      'data-page-phase',
      'idle',
    );
    await expect.poll(scroll).toBe(position);
    if (!compact)
      expect(await workspace.evaluate((element) => element.scrollTop)).toBe(0);
    await testInfo.attach('settings-scroll', {
      body: JSON.stringify({
        viewport,
        scroll: await scroll(),
        url: page.url(),
      }),
      contentType: 'application/json',
    });
    await testInfo.attach('settings-scroll-view', {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('shorter settings sections clamp scroll to the available content', async ({
  page,
}, testInfo) => {
  // Only the compact layout scrolls the workspace and keeps its position
  // across settings sections. Links are clicked in place: an actionable click
  // would first scroll the stacked navigation back into view.
  await page.setViewportSize({ width: 390, height: 844 });
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'devices',
  });
  await page.goto('/');
  await expect(page.getByText('Browser 39', { exact: true })).toBeAttached();
  const workspace = page.locator('.workspace-scroll');
  await workspace.evaluate((element) => {
    element.scrollTop = 1200;
  });
  expect(await workspace.evaluate((element) => element.scrollTop)).toBe(1200);
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('link', { name: 'Online accounts', exact: true })
    .dispatchEvent('click');
  await expect(
    page.getByRole('button', { name: 'Connect YouTube', exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      workspace.evaluate(
        (element) =>
          element.scrollTop ===
          Math.min(1200, element.scrollHeight - element.clientHeight),
      ),
    )
    .toBe(true);
  await testInfo.attach('shorter-settings-scroll', {
    body: JSON.stringify(
      await workspace.evaluate((element) => ({
        scroll: element.scrollTop,
        maxScroll: element.scrollHeight - element.clientHeight,
      })),
    ),
    contentType: 'application/json',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const manualScroll of [false, true]) {
  test(`delayed settings loading ${manualScroll ? 'respects manual scrolling' : 'restores the retained scroll'}`, async ({
    page,
  }, testInfo) => {
    // Only the compact layout scrolls the workspace and keeps its position
    // across settings sections. Scroll past the stacked navigation so the
    // loading section is shorter than the retained position.
    await page.setViewportSize({ width: 390, height: 844 });
    const fixture = await installUiFixture(page, { role: 'admin' });
    const devices = responseGate();
    await page.route('**/api/v1/auth/sessions', async (route) => {
      await devices.promise;
      await route.fallback();
    });
    await page.goto('/');
    await expect(
      page.getByLabel('New password', { exact: true }),
    ).toBeVisible();
    const workspace = page.locator('.workspace-scroll');
    const scroll = () => workspace.evaluate((element) => element.scrollTop);
    const retained = 1200;
    await workspace.evaluate((element, top) => {
      element.scrollTop = top;
    }, retained);
    expect(await scroll()).toBe(retained);
    await page
      .getByRole('navigation', { name: 'Settings navigation' })
      .getByRole('link', { name: 'Devices', exact: true })
      .dispatchEvent('click');
    await expect(
      page.locator('[role="status"][aria-label="Loading devices"]'),
    ).toBeAttached();
    await expect(page.locator('[data-page-region="settings"]')).toHaveCSS(
      'opacity',
      '0',
    );
    await expect.poll(scroll).toBeLessThan(retained);
    let expected = retained;
    if (manualScroll) {
      const before = await scroll();
      await workspace.hover();
      await page.mouse.wheel(0, -12);
      await expect.poll(scroll).toBe(Math.max(0, before - 12));
      expected = await scroll();
    }
    devices.release();
    await expect(page.getByText('Browser 39', { exact: true })).toBeAttached();
    await workspace.evaluate(
      () => new Promise((resolve) => requestAnimationFrame(resolve)),
    );
    await expect.poll(scroll).toBe(expected);
    await testInfo.attach('delayed-settings-scroll', {
      body: JSON.stringify({ manualScroll, scroll: await scroll(), expected }),
      contentType: 'application/json',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('rapid settings switches keep the latest scroll while a section first loads', async ({
  page,
}, testInfo) => {
  // Only the compact layout scrolls the workspace and keeps its position
  // across settings sections.
  await page.setViewportSize({ width: 390, height: 844 });
  const fixture = await installUiFixture(page, { role: 'admin' });
  await page.goto('/');
  await expect(page.getByLabel('New password', { exact: true })).toBeVisible();
  const chunks = responseGate();
  const devices = responseGate();
  await page.route('**/assets/*.js', async (route) => {
    await chunks.promise;
    await route.continue();
  });
  await page.route('**/api/v1/auth/sessions', async (route) => {
    await devices.promise;
    await route.fallback();
  });
  const workspace = page.locator('.workspace-scroll');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await workspace.evaluate((element) => {
    element.scrollTop = 80;
  });
  await nav.getByRole('link', { name: 'Devices', exact: true }).click();
  await expect(page.locator('.settings-header > h1')).toHaveText('Devices');
  await nav.getByRole('link', { name: 'Account', exact: true }).click();
  await expect(page.getByLabel('New password', { exact: true })).toBeVisible();
  await workspace.hover();
  await page.mouse.wheel(0, -200);
  await expect
    .poll(() => workspace.evaluate((element) => element.scrollTop))
    .toBe(0);
  await nav.getByRole('link', { name: 'Devices', exact: true }).click();
  chunks.release();
  await expect(
    page.getByRole('heading', { name: 'Connect a TV', includeHidden: true }),
  ).toBeAttached();
  devices.release();
  await expect(page.getByText('Browser 39', { exact: true })).toBeAttached();
  await workspace.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(resolve)),
  );
  await expect
    .poll(() => workspace.evaluate((element) => element.scrollTop))
    .toBe(0);
  await testInfo.attach('rapid-settings-scroll', {
    body: JSON.stringify({
      scroll: await workspace.evaluate((element) => element.scrollTop),
      url: page.url(),
    }),
    contentType: 'application/json',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
