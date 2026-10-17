import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { responseGate } from './helpers/response-gate';

test.afterEach(async ({ page }) => {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
});

async function installSeries(
  page: Page,
  role: 'admin' | 'user' = 'admin',
  blocked = false,
) {
  const fixture = await installUiFixture(page, {
    role,
    section: 'Shows',
    preserveNavigation: true,
  });
  const series = {
    id: 'series',
    kind: 'show',
    title: 'Fixture series',
    parent_id: null as string | null,
    sort_number: null as number | null,
    available: false,
    metadata: {},
  };
  const items = [series];
  for (const season of [1, 2]) {
    items.push({
      ...series,
      id: `season-${season}`,
      kind: 'season',
      title: `Season ${season}`,
      parent_id: series.id,
      sort_number: season,
    });
    for (const episode of [1, 2])
      items.push({
        ...series,
        id: `episode-${season}-${episode}`,
        kind: 'episode',
        title: `Episode ${season}.${episode}`,
        parent_id: `season-${season}`,
        sort_number: episode,
        available: true,
      });
  }
  const descendants = (id: string): typeof items => [
    ...items.filter((item) => item.id === id),
    ...items
      .filter((item) => item.parent_id === id)
      .flatMap((item) => descendants(item.id)),
  ];
  await page.route('**/api/v1/catalog?**', (route) => {
    const query = new URL(route.request().url()).searchParams;
    return route.fulfill({
      json: {
        items: items.filter((item) =>
          query.has('parent')
            ? item.parent_id === query.get('parent')
            : item.kind === query.get('kind'),
        ),
      },
    });
  });
  await page.route('**/api/v1/catalog/*', (route) => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1);
    const item = items.find((item) => item.id === id);
    return item
      ? route.fulfill({ json: { ...item, files: [], keep: false } })
      : route.fallback();
  });
  await page.route('**/api/v1/catalog/*/state', (route) =>
    route.fulfill({
      json: { favorite: false, watch_later: false, watched: false },
    }),
  );
  await page.route('**/api/v1/catalog/series/seasons', (route) =>
    route.fulfill({
      json: {
        seasons: items
          .filter((item) => item.kind === 'season')
          .map((season) => ({
            number: season.sort_number,
            id: season.id,
            title: season.title,
            status: null,
            watched: false,
            episodes: items
              .filter((item) => item.parent_id === season.id)
              .map((episode) => ({
                id: episode.id,
                season: season.sort_number,
                number: episode.sort_number,
                title: episode.title,
                overview: null,
                air_date: null,
                runtime: null,
                available: episode.available,
                watched: false,
                position: null,
                duration: null,
                status: null,
              })),
          })),
      },
    }),
  );
  const operations = new Map<string, string>();
  const executed: string[] = [];
  await page.route('**/api/v1/admin/media/operations', (route) => {
    const { media_id } = route.request().postDataJSON();
    const id = `operation-${operations.size}`;
    operations.set(id, media_id);
    return route.fulfill({
      json: {
        id,
        files: descendants(media_id).filter((item) => item.available).length,
      },
    });
  });
  await page.route('**/api/v1/admin/media/operations/*/execute', (route) => {
    if (blocked)
      return route.fulfill({
        status: 409,
        json: {
          error: { message: 'Media is protected by Keep' },
        },
      });
    const id = new URL(route.request().url()).pathname.split('/').at(-2)!;
    executed.push(operations.get(id)!);
    for (const item of descendants(operations.get(id)!)) item.available = false;
    return route.fulfill({ json: { state: 'complete' } });
  });
  return { ...fixture, items, executed };
}

async function openSeries(page: Page) {
  await page.goto('/');
  await page
    .getByRole('heading', { name: 'Fixture series', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { level: 1, name: 'Fixture series' }),
  ).toBeVisible();
}

async function deletion(page: Page, scope: 'season' | 'series') {
  await page
    .getByRole('button', {
      name: `More actions for ${scope === 'season' ? 'Season 1' : 'Fixture series'}`,
      exact: true,
    })
    .click();
  await page
    .getByRole('menuitem', { name: `Delete ${scope} files…`, exact: true })
    .click();
  return page.getByRole('dialog', {
    name: new RegExp(`Delete ${scope} files`),
  });
}

const playable = (page: Page, number: number, title: string) =>
  page.getByRole('button', {
    name: `Play episode ${number}: ${title}`,
    exact: true,
  });

for (const scope of ['season', 'series'] as const) {
  test(`delete a ${scope} keeps its catalogue and removes only its files`, async ({
    page,
  }, testInfo) => {
    const fixture = await installSeries(page);
    await openSeries(page);
    let dialog = await deletion(page, scope);
    await expect(
      dialog.getByText(`Delete ${scope === 'season' ? 2 : 4} files`, {
        exact: false,
      }),
    ).toBeVisible();
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    dialog = await deletion(page, scope);
    await dialog
      .getByRole('button', { name: `Delete ${scope} files`, exact: true })
      .click();
    await expect(dialog).toHaveCount(0);
    await page.reload();
    await expect(
      page.getByRole('heading', { level: 1, name: 'Fixture series' }),
    ).toBeVisible();
    await expect(page.getByText('Episode 1.1')).toBeVisible();
    await expect(playable(page, 1, 'Episode 1.1')).toHaveCount(0);
    await page.getByRole('tab', { name: 'Season 2', exact: true }).click();
    await expect(page.getByText('Episode 2.1')).toBeVisible();
    await expect(playable(page, 1, 'Episode 2.1')).toHaveCount(
      scope === 'season' ? 1 : 0,
    );
    await testInfo.attach('retained-series', {
      body: await page.screenshot({ animations: 'disabled' }),
      contentType: 'image/png',
    });
    await testInfo.attach('deletion-result', {
      body: JSON.stringify({ scope, catalogue: fixture.items }),
      contentType: 'application/json',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('a deletion prepared after its dialog closed is never offered', async ({
  page,
}, testInfo) => {
  const fixture = await installSeries(page);
  await openSeries(page);
  const held = responseGate();
  let requested = false;
  await page.route('**/api/v1/admin/media/operations', async (route) => {
    requested = true;
    await held.promise;
    await route.fallback();
  });
  try {
    const dialog = await deletion(page, 'series');
    await expect.poll(() => requested).toBe(true);
    await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    const response = page.waitForResponse('**/api/v1/admin/media/operations');
    held.release();
    await (await response).finished();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect(playable(page, 1, 'Episode 1.1')).toBeVisible();
    const season = await deletion(page, 'season');
    await expect(
      season.getByText('Delete 2 files', { exact: false }),
    ).toBeVisible();
    await season
      .getByRole('button', { name: 'Delete season files', exact: true })
      .click();
    await expect(season).toHaveCount(0);
    expect(fixture.executed).toEqual(['season-1']);
    await testInfo.attach('pending-deletion-cleared', {
      body: await page.screenshot({ animations: 'disabled' }),
      contentType: 'image/png',
    });
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  } finally {
    held.release();
  }
});

test('a rejected season deletion shows the error and keeps episodes playable', async ({
  page,
}, testInfo) => {
  const fixture = await installSeries(page, 'admin', true);
  await openSeries(page);
  const dialog = await deletion(page, 'season');
  await dialog
    .getByRole('button', { name: 'Delete season files', exact: true })
    .click();
  await expect(
    dialog.getByRole('alert').filter({ hasText: 'Media is protected by Keep' }),
  ).toBeVisible();
  await testInfo.attach('rejected-deletion', {
    body: await page.screenshot({ animations: 'disabled' }),
    contentType: 'image/png',
  });
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(playable(page, 1, 'Episode 1.1')).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('members can browse series and seasons without administration actions', async ({
  page,
}, testInfo) => {
  const fixture = await installSeries(page, 'user');
  await openSeries(page);
  await expect(playable(page, 1, 'Episode 1.1')).toBeVisible();
  // The series page shows Watch Later and Favorite itself, leaving members no menu.
  await expect(
    page.getByRole('button', {
      name: 'More actions for Fixture series',
      exact: true,
    }),
  ).toHaveCount(0);
  await page
    .getByRole('button', { name: 'More actions for Season 1', exact: true })
    .click();
  const menu = page.getByRole('menu');
  await expect(
    menu.getByRole('menuitem', { name: 'Add to Watch Later' }),
  ).toBeVisible();
  await expect(menu.getByText('Administration')).toHaveCount(0);
  await expect(menu.getByRole('menuitem', { name: /Delete/ })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await testInfo.attach('member-series', {
    body: await page.screenshot({ animations: 'disabled' }),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
