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

// Page transitions finish before a screenshot is worth keeping.
const settled = (page: Page) =>
  expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );

async function view(page: Page, role: 'admin' | 'user') {
  await page.goto('about:blank');
  await page.request.post('/__mock/viewer', { data: { role } });
}

test('a listener finds an artist, requests albums and monitors new releases', async ({
  page,
}, testInfo) => {
  const errors = track(page);
  await view(page, 'user');
  await page.goto('/#discover');
  const tabs = page.getByRole('navigation', { name: 'Discover navigation' });
  await tabs.getByRole('button', { name: 'Music' }).click();
  for (const row of [
    'More from your artists',
    'Similar to Beyond the Horizon',
    'Trending this week',
  ])
    await expect(page.getByRole('region', { name: row })).toBeVisible();
  // An album without a cover still names itself.
  await expect(
    page
      .getByRole('region', { name: 'Similar to Beyond the Horizon' })
      .locator('[data-cover-fallback]')
      .first(),
  ).toContainText('Mira Sol');
  await page.screenshot({ path: testInfo.outputPath('music-home.png') });

  await page
    .getByRole('searchbox', { name: 'Search albums and artists' })
    .or(page.getByRole('textbox', { name: 'Search albums and artists' }))
    .fill('northern choir');
  const top = page.getByRole('article', { name: 'Northern Choir' });
  await expect(top).toContainText('Artist · 5 albums');
  await expect(
    page.getByRole('region', { name: 'Albums by Northern Choir' }),
  ).toBeVisible();
  await top.getByRole('button', { name: 'Monitor new releases' }).click();
  await expect(top).toContainText('Monitoring pending approval');
  await page.screenshot({ path: testInfo.outputPath('music-search.png') });

  await top.getByRole('button', { name: 'Open artist' }).click();
  await expect(
    page.getByRole('heading', { level: 1, name: 'Northern Choir' }),
  ).toBeVisible();
  await expect(page).toHaveURL(/#discover\/artist\/0a000000-/);
  await expect(page.getByText('Monitoring pending approval')).toBeVisible();
  const albums = page.getByRole('region', { name: 'Albums' });
  const picks = albums.getByRole('button', { name: /^Select / });
  await picks.nth(0).click();
  await picks.nth(0).click();
  await expect(albums.getByRole('button', { name: /^Deselect / })).toHaveCount(
    2,
  );
  await page.getByRole('button', { name: 'Request 2 albums' }).click();
  await expect(albums.getByText('Pending approval')).toHaveCount(2);
  await expect(albums.getByRole('button', { name: /^Select / })).toHaveCount(3);

  const other = page.getByRole('region', { name: 'Other releases' });
  await other.getByRole('button', { name: /Other releases/ }).click();
  await expect(other).toContainText('Collected Works');
  await expect(other).toContainText(
    "Lidarr's Standard metadata profile leaves these out.",
  );

  const last = albums.getByRole('article').last();
  const title = (await last.getAttribute('aria-label'))!;
  await last.getByRole('button', { name: title, exact: true }).click();
  const sheet = page.getByRole('dialog', { name: title });
  await expect(sheet.getByRole('list', { name: 'Tracks' })).toBeVisible();
  await sheet.getByRole('button', { name: 'Request album' }).click();
  await expect(sheet).toContainText('Pending approval');
  await page.screenshot({ path: testInfo.outputPath('music-album.png') });
  await sheet.getByRole('button', { name: 'Close' }).click();
  await expect(last).toContainText('Pending approval');
  await page.screenshot({ path: testInfo.outputPath('music-artist.png') });

  await page.getByRole('button', { name: 'Back to discover' }).click();
  await expect(tabs.getByRole('button', { name: 'Music' })).toHaveAttribute(
    'aria-current',
    'page',
  );
  expect(errors).toEqual([]);
});

test('an administrator approves an artist at once and fills gaps from the library', async ({
  page,
}, testInfo) => {
  const errors = track(page);
  await view(page, 'admin');
  await page.goto('/#requests');
  const music = page.getByRole('region', { name: 'Music requests' });
  const group = music.getByRole('article', { name: 'Harbor Lights requests' });
  await expect(group).toContainText(
    '2 albums and new releases · Pending approval',
  );
  await expect(group).toContainText('Monitor new releases');
  await music.screenshot({ path: testInfo.outputPath('music-requests.png') });
  await group.getByRole('button', { name: 'Approve all' }).click();
  await expect(group).toHaveCount(0);
  await expect(
    music.getByRole('article', { name: 'Harbor Lights', exact: true }),
  ).toContainText('Monitoring new releases');

  // The library page shows without waiting for the discography.
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route(/\/acquisition\/music\/artists\/[^/?]+\?/, async (route) => {
    await held;
    await route.continue();
  });
  await page.goto('/#library/music/artist-0');
  await settled(page);
  await expect(
    page.getByRole('heading', { level: 1, name: 'Beyond the Horizon' }),
  ).toBeVisible();
  const gaps = page.getByRole('region', { name: 'Not in your library' });
  await expect(gaps).toHaveCount(0);
  release();
  await expect(gaps.getByRole('article')).toHaveCount(4);
  await expect(gaps).toContainText('Missing');
  await gaps.scrollIntoViewIfNeeded();
  await settled(page);
  await gaps.screenshot({ path: testInfo.outputPath('library-gaps.png') });
  await gaps.getByRole('button', { name: 'Full discography' }).click();
  await expect(page).toHaveURL(/#discover\/artist\/0a000000-/);
  await expect(
    page.getByRole('button', { name: 'Back to discover' }),
  ).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Beyond the Horizon',
  );
  const monitoring = page.getByRole('button', {
    name: 'Monitoring new releases',
  });
  await settled(page);
  await page.screenshot({
    path: testInfo.outputPath('music-artist-admin.png'),
  });
  await monitoring.click();
  await expect(
    page.getByRole('button', { name: 'Monitor new releases' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Open in library' }).click();
  await expect(page).toHaveURL(/#library\/music\/artist-0/);
  expect(errors).toEqual([]);
});
