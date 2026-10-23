import { expect, test } from '@playwright/test';
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
  await expect(poster).toContainText('6 columns');
  await expect(video).toContainText('6 columns');
  await poster.getByRole('button', { name: 'Larger poster cards' }).click();
  await video.getByRole('button', { name: 'Smaller video cards' }).click();
  await expect(poster).toContainText('5 columns');
  await expect(video).toContainText('7 columns');
  await expect
    .poll(
      () =>
        fixture.writes.filter((write) => write.path === '/me/appearance')
          .length,
    )
    .toBe(2);
  await page.reload();
  await expect(poster).toContainText('5 columns');
  await expect(video).toContainText('7 columns');
  const nav = page.getByRole('navigation', { name: 'Main navigation' });
  const evidence: { section: string; before: number; after: number }[] = [];
  let posterColumns = 5,
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
  for (const group of [poster, video]) {
    const larger = group.getByRole('button', { name: /^Larger / });
    const smaller = group.getByRole('button', { name: /^Smaller / });
    while (await larger.isEnabled()) await larger.click();
    await expect(group).toContainText('3 columns');
    await expect(larger).toBeDisabled();
    while (await smaller.isEnabled()) await smaller.click();
    await expect(group).toContainText('12 columns');
    await expect(smaller).toBeDisabled();
  }
  // Rapid clicks save their last value once input pauses.
  await expect
    .poll(fixture.savedAppearance)
    .toMatchObject({ poster_columns: 12, video_columns: 12 });
  await page.reload();
  await expect(poster).toContainText('12 columns');
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
