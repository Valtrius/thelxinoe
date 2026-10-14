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
    });
    for (const episode of [1, 2])
      items.push({
        ...series,
        id: `episode-${season}-${episode}`,
        kind: 'episode',
        title: `Episode ${season}.${episode}`,
        parent_id: `season-${season}`,
        available: true,
      });
  }
  const descendants = (id: string): typeof items => [
    ...items.filter((item) => item.id === id),
    ...items
      .filter((item) => item.parent_id === id)
      .flatMap((item) => descendants(item.id)),
  ];
  await page.route('**/api/v1/catalog/roots', (route) =>
    route.fulfill({ json: { items: [], media_mount: '/media' } }),
  );
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
      ? route.fulfill({ json: { ...item, files: [] } })
      : route.fallback();
  });
  const operations = new Map<string, string>();
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
    for (const item of descendants(operations.get(id)!)) item.available = false;
    return route.fulfill({ json: { state: 'complete' } });
  });
  return { ...fixture, items };
}

for (const scope of ['season', 'series'] as const) {
  test(`delete a ${scope} keeps its catalogue and removes only its files`, async ({
    page,
  }, testInfo) => {
    const fixture = await installSeries(page);
    await page.goto('/');
    await page
      .getByRole('heading', { name: 'Fixture series', exact: true })
      .click();
    if (scope === 'season')
      await page
        .getByRole('heading', { name: 'Season 1', exact: true })
        .click();
    const prepare = page.getByRole('button', {
      name: `Delete ${scope} files`,
      exact: true,
    });
    await prepare.click();
    await expect(
      page.getByText(`${scope === 'season' ? 2 : 4} file(s)`, { exact: false }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(
      page.getByRole('button', { name: 'Confirm delete', exact: true }),
    ).toHaveCount(0);
    await prepare.click();
    await page
      .getByRole('button', { name: 'Confirm delete', exact: true })
      .click();
    await expect(
      page.getByRole('status').filter({ hasText: 'Operation completed.' }),
    ).toBeVisible();
    await page.reload();
    if (scope === 'series')
      await page
        .getByRole('heading', { name: 'Season 1', exact: true })
        .click();
    await expect(
      page.getByRole('heading', { name: 'Episode 1.1', exact: true }),
    ).toBeVisible();
    await expect(
      page
        .locator('article')
        .filter({
          has: page.getByRole('heading', { name: 'Episode 1.1', exact: true }),
        })
        .getByText('episode · Unavailable', { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Play Episode 1.1', exact: true }),
    ).toHaveCount(0);
    await page.getByRole('button', { name: 'Season 1', exact: true }).click();
    await page.getByRole('heading', { name: 'Season 2', exact: true }).click();
    await expect(
      page.getByRole('heading', { name: 'Episode 2.1', exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Play Episode 2.1', exact: true }),
    ).toHaveCount(scope === 'season' ? 1 : 0);
    await page.getByRole('button', { name: 'Season 2', exact: true }).click();
    await page
      .getByRole('button', { name: 'Fixture series', exact: true })
      .click();
    await expect(
      page.getByRole('heading', { name: 'Fixture series', exact: true }),
    ).toBeVisible();
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

test('a pending deletion cannot follow navigation to another season or series', async ({
  page,
}, testInfo) => {
  const fixture = await installSeries(page);
  await page.goto('/');
  await page
    .getByRole('heading', { name: 'Fixture series', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Delete series files', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Confirm delete', exact: true }),
  ).toBeVisible();
  await page.getByRole('heading', { name: 'Season 1', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Confirm delete', exact: true }),
  ).toHaveCount(0);
  const held = responseGate();
  let requested = false;
  await page.route('**/api/v1/admin/media/operations', async (route) => {
    requested = true;
    await held.promise;
    await route.fallback();
  });
  try {
    await page
      .getByRole('button', { name: 'Delete season files', exact: true })
      .click();
    await expect.poll(() => requested).toBe(true);
    await page.getByRole('button', { name: 'Season 1', exact: true }).click();
    await page.getByRole('heading', { name: 'Season 2', exact: true }).click();
    const response = page.waitForResponse('**/api/v1/admin/media/operations');
    held.release();
    await (await response).finished();
    await expect(
      page.getByRole('button', { name: 'Delete season files', exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole('button', { name: 'Confirm delete', exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole('button', { name: 'Play Episode 2.1', exact: true }),
    ).toBeVisible();
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
  await page.goto('/');
  await page
    .getByRole('heading', { name: 'Fixture series', exact: true })
    .click();
  await page.getByRole('heading', { name: 'Season 1', exact: true }).click();
  await page
    .getByRole('button', { name: 'Delete season files', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Confirm delete', exact: true })
    .click();
  await expect(
    page.getByRole('status').filter({ hasText: 'Media is protected by Keep' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Play Episode 1.1', exact: true }),
  ).toBeVisible();
  await testInfo.attach('rejected-deletion', {
    body: await page.screenshot({ animations: 'disabled' }),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('members can browse series and seasons without deletion controls', async ({
  page,
}, testInfo) => {
  const fixture = await installSeries(page, 'user');
  await page.goto('/');
  await page
    .getByRole('heading', { name: 'Fixture series', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Delete series files', exact: true }),
  ).toHaveCount(0);
  await page.getByRole('heading', { name: 'Season 1', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Episode 1.1', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Manage media', exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Delete season files', exact: true }),
  ).toHaveCount(0);
  await testInfo.attach('member-season', {
    body: await page.screenshot({ animations: 'disabled' }),
    contentType: 'image/png',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
