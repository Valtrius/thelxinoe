import { expect, test, type Locator, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('offscreen artwork skips rendering without changing a large library grid', async ({
  page,
}) => {
  await installUiFixture(page, { section: 'Movies' });
  await page.route('**/api/v1/catalog?**', (route) =>
    route.fulfill({
      json: {
        items: Array.from({ length: 150 }, (_, index) => ({
          id: `movie-${index}`,
          kind: 'movie',
          title:
            index % 2
              ? `Movie ${index}`
              : `A much longer movie title that wraps across several lines ${index}`,
          available: true,
          year: 2026,
        })),
        total: 150,
      },
    }),
  );
  await page.goto('/');
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
  const cards = page.locator('.library-tile');
  await expect(cards).toHaveCount(150);
  const last = cards.last();
  const skipped = () =>
    last
      .locator('.tile-art svg')
      .evaluate(
        (node) => !node.checkVisibility({ contentVisibilityAuto: true }),
      );
  await expect.poll(skipped).toBe(true);
  const geometry = () =>
    page.locator('[data-card-grid]').evaluate((grid) => ({
      height: grid.getBoundingClientRect().height,
      cards: [...grid.children].map((card) => ({
        width: card.getBoundingClientRect().width,
        height: card.getBoundingClientRect().height,
      })),
    }));
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await expect
      .poll(() =>
        cards
          .first()
          .boundingBox()
          .then((box) => box!.width),
      )
      .toBeGreaterThan(0);
    // Wait for the layout's own resize animation before comparing containment modes.
    await expect
      .poll(async () => {
        const a = await geometry();
        await page.evaluate(
          () =>
            new Promise((resolve) =>
              requestAnimationFrame(() => requestAnimationFrame(resolve)),
            ),
        );
        return JSON.stringify(a) === JSON.stringify(await geometry());
      })
      .toBe(true);
    const contained = await geometry();
    await page
      .locator('.tile-art')
      .evaluateAll((nodes) =>
        nodes.forEach(
          (node) => ((node as HTMLElement).style.contentVisibility = 'visible'),
        ),
      );
    expect(await geometry()).toEqual(contained);
    await page
      .locator('.tile-art')
      .evaluateAll((nodes) =>
        nodes.forEach((node) =>
          (node as HTMLElement).style.removeProperty('content-visibility'),
        ),
      );
    await last.scrollIntoViewIfNeeded();
    await expect.poll(skipped).toBe(false);
    await expect(last.locator('.tile-art')).toBeInViewport();
    await last.locator('.card-primary').focus();
    await expect(last.locator('.card-primary')).toBeFocused();
    await cards.first().scrollIntoViewIfNeeded();
  }
});
import { installDensityData } from '../scripts/density-fixtures.mjs';

test('poster and video density stay independent across settings, all views and reloads', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { preserveNavigation: true });
  await installDensityData(page);
  await page.goto('/');
  const poster = page.getByRole('group', {
    name: 'Poster density',
    exact: true,
  });
  const video = page.getByRole('group', { name: 'Video density', exact: true });
  await expect(poster).toContainText('9 columns');
  await expect(video).toContainText('6 columns');
  await poster.getByRole('button', { name: 'Larger poster cards' }).click();
  await video.getByRole('button', { name: 'Smaller video cards' }).click();
  await expect(poster).toContainText('8 columns');
  await expect(video).toContainText('7 columns');
  await expect
    .poll(
      () =>
        fixture.writes.filter((write) => write.path === '/me/appearance')
          .length,
    )
    .toBe(2);
  await page.reload();
  await expect(poster).toContainText('8 columns');
  await expect(video).toContainText('7 columns');
  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  const evidence: { section: string; before: number; after: number }[] = [];
  let posterColumns = 8,
    videoColumns = 7;
  for (const section of [
    'Movies',
    'Shows',
    'Music',
    'YouTube',
    'Twitch',
    'Kick',
  ]) {
    const posters = ['Movies', 'Shows', 'Music'].includes(section);
    const before = posters ? posterColumns : videoColumns;
    await nav.getByRole('link', { name: section, exact: true }).click();
    const grid = page
      .locator('[data-card-grid]')
      .filter({ has: page.getByRole('button') })
      .first();
    await expect(grid).toBeVisible();
    const columns = () =>
      grid.evaluate(
        (element) =>
          getComputedStyle(element).gridTemplateColumns.split(' ').length,
      );
    await expect.poll(columns).toBe(before);
    const card = grid.locator(':scope > *').first();
    await expect(card).toBeVisible();
    const width = (await card.boundingBox())!.width;
    await card.hover();
    const writes = fixture.writes.filter(
      (write) => write.path === '/me/appearance',
    ).length;
    await page.mouse.wheel(0, 100);
    expect(
      fixture.writes.filter((write) => write.path === '/me/appearance'),
    ).toHaveLength(writes);
    await card.hover();
    // Movies and YouTube zoom in; the other views zoom out.
    const delta = section === 'Movies' || section === 'YouTube' ? -1 : 1;
    await page.keyboard.down('Control');
    await page.mouse.wheel(0, delta * 100);
    await page.keyboard.up('Control');
    const after = before + delta;
    await expect.poll(columns).toBe(after);
    await expect
      .poll(async () => Math.sign((await card.boundingBox())!.width - width))
      .toBe(-delta);
    if (posters) posterColumns = after;
    else videoColumns = after;
    await expect
      .poll(
        () =>
          fixture.writes.filter((write) => write.path === '/me/appearance')
            .length,
      )
      .toBe(writes + 1);
    evidence.push({ section, before, after });
    await testInfo.attach(`${section}-density`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await expect(poster).toContainText(`${posterColumns} columns`);
    await expect(video).toContainText(`${videoColumns} columns`);
  }
  for (const [group, min, max] of [
    [poster, 5, 18],
    [video, 3, 12],
  ] as const) {
    const larger = group.getByRole('button', { name: /^Larger / });
    const smaller = group.getByRole('button', { name: /^Smaller / });
    while (await larger.isEnabled()) await larger.click();
    await expect(group).toContainText(`${min} columns`);
    await expect(larger).toBeDisabled();
    while (await smaller.isEnabled()) await smaller.click();
    await expect(group).toContainText(`${max} columns`);
    await expect(smaller).toBeDisabled();
  }
  // Rapid clicks save their last value once input pauses.
  await expect
    .poll(fixture.savedAppearance)
    .toMatchObject({ poster_columns: 18, video_columns: 12 });
  await page.reload();
  await expect(poster).toContainText('18 columns');
  await expect(video).toContainText('12 columns');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
    )
    .toBe(true);
  await poster.scrollIntoViewIfNeeded();
  await testInfo.attach('density-settings-mobile', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await testInfo.attach('density-transitions', {
    body: JSON.stringify(evidence, null, 2),
    contentType: 'application/json',
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

const shownFontSize = (element: Locator) =>
  element.evaluate((node) => {
    let zoom = 1;
    for (
      let parent: Element | null = node;
      parent;
      parent = parent.parentElement
    )
      zoom *= Number(getComputedStyle(parent).zoom);
    return parseFloat(getComputedStyle(node).fontSize) * zoom;
  });

test('nine posters show their titles at the size of six videos', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { preserveNavigation: true });
  await installDensityData(page);
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  const titleSize = async (section: string) => {
    await nav.getByRole('link', { name: section, exact: true }).click();
    const title = page.locator('[data-card-grid] article h3').first();
    await expect(title).toBeVisible();
    return shownFontSize(title);
  };
  const poster = await titleSize('Movies');
  const video = await titleSize('YouTube');
  expect(poster / video).toBeGreaterThan(0.9);
  expect(poster / video).toBeLessThan(1.1);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test.describe('on a high-density display', () => {
  test.use({ deviceScaleFactor: 2 });

  async function openPoster(page: Page) {
    await installUiFixture(page, { section: 'Movies' });
    await page.route('**/api/v1/catalog?**', (route) =>
      route.fulfill({
        json: {
          items: [
            {
              id: 'movie',
              kind: 'movie',
              title: 'Movie',
              available: true,
              artwork_url: '/api/v1/catalog/movie/artwork?grant=stable',
            },
          ],
          total: 1,
        },
      }),
    );
    await page.route('**/api/v1/catalog/movie/artwork?*', (route) => {
      const width = Number(
        new URL(route.request().url()).searchParams.get('width'),
      );
      return route.fulfill({
        contentType: 'image/svg+xml',
        body: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${width * 1.5}"><rect width="100%" height="100%" fill="#555"/></svg>`,
      });
    });
    await page.goto('/');
    const image = page.locator('.library-tile img[data-art]');
    const selectedWidth = () =>
      image.evaluate((img) =>
        Number(
          new URL((img as HTMLImageElement).currentSrc).searchParams.get(
            'width',
          ),
        ),
      );
    await expect.poll(selectedWidth).toBe(500);
    return { image, selectedWidth };
  }

  test('library posters select a larger artwork variant at fewer columns', async ({
    page,
  }) => {
    const { image, selectedWidth } = await openPoster(page);
    // Five columns on this viewport ask for 576 device pixels, past that file.
    await image.hover();
    await page.keyboard.down('Control');
    for (let step = 0; step < 4; step += 1) await page.mouse.wheel(0, -100);
    await page.keyboard.up('Control');
    await expect.poll(selectedWidth).toBe(1000);
  });

  test('library posters select their artwork from the columns a narrow window shows', async ({
    page,
  }) => {
    const { selectedWidth } = await openPoster(page);
    // This window falls back to three columns, whatever density is saved.
    await page.setViewportSize({ width: 840, height: 900 });
    await expect
      .poll(() =>
        page
          .locator('[data-card-grid]')
          .evaluate(
            (grid) =>
              getComputedStyle(grid).gridTemplateColumns.split(' ').length,
          ),
      )
      .toBe(3);
    await expect.poll(selectedWidth).toBe(1000);
  });
});
