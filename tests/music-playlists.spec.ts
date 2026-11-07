import { expect, test, type Locator, type Page } from '@playwright/test';
import { hold } from './helpers/touch';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

const frame = (page: Page) => page.locator('[data-youtube-watchlist-frame]');
const rowsOf = (scope: Locator) => scope.locator('[data-list-item-key]');
const keys = (rows: Locator) =>
  rows.evaluateAll((elements) =>
    elements.map((element) => (element as HTMLElement).dataset.listItemKey!),
  );
const button = (scope: Page | Locator, name: string | RegExp) =>
  scope.getByRole('button', { name, exact: typeof name === 'string' });
const picker = (scope: Page | Locator, name: string) =>
  scope.getByRole('button', { name: new RegExp(`^${name}`) }).first();

async function openPlaylists(page: Page) {
  await page.goto('/#library/music');
  await button(page, 'Open playlists').click();
  await expect(rowsOf(frame(page)).first()).toBeVisible();
}
async function choose(page: Page, from: string, to: string) {
  await picker(frame(page), from).click();
  await frame(page)
    .getByRole('dialog', { name: 'Playlists' })
    .getByRole('button', { name: new RegExp(`^${to}`) })
    .click();
  await expect(picker(frame(page), to)).toBeVisible();
}
const playlist = async (
  request: Page['request'],
  id: string,
): Promise<{
  access: string;
  name: string;
  sort_mode: string;
  items: { id: string; title: string }[];
}> => (await request.get(`/api/v1/playlists/${id}`)).json();

test('the sidebar replaces the Playlists page and stays open across Music pages', async ({
  page,
}, info) => {
  await page.goto('/#library/music');
  await expect(
    page.getByRole('link', { name: 'Music', exact: true }),
  ).toBeVisible();
  await expect(page.getByRole('link', { name: 'Playlists' })).toHaveCount(0);
  await expect(frame(page).locator('aside')).toBeHidden();

  await button(page, 'Open playlists').click();
  const sidebar = frame(page);
  await expect(picker(sidebar, 'Evening listening')).toBeVisible();
  await expect(rowsOf(sidebar)).toHaveCount(12);
  await choose(page, 'Evening listening', 'On the road');
  await expect(page).toHaveURL(/playlist=playlist-2/);
  await expect(rowsOf(sidebar)).toHaveCount(10);
  await page.screenshot({ path: info.outputPath('home.png') });

  // The content sits beside the sidebar, with nothing under it.
  const content = (await page.locator('[data-music-scroll]').boundingBox())!;
  const beside = (await sidebar.boundingBox())!;
  expect(content.x + content.width).toBeCloseTo(beside.x, 0);

  await page.locator('[data-card-grid] .card-primary').first().click();
  await expect(button(page, /^Back to/)).toBeVisible();
  await expect(picker(sidebar, 'On the road')).toBeVisible();
  await expect(rowsOf(sidebar)).toHaveCount(10);
  await expect(button(page, 'Close playlists')).toBeVisible();
  await page.screenshot({ path: info.outputPath('artist.png') });

  await page.reload();
  await expect(picker(frame(page), 'On the road')).toBeVisible();
  await expect(rowsOf(frame(page))).toHaveCount(10);

  await button(page, 'Close playlists').click();
  await expect(button(page, 'Open playlists')).toBeVisible();
  await expect(sidebar).toHaveJSProperty('offsetWidth', 0);
  await expect(page).not.toHaveURL(/playlist=/);
  await page.reload();
  await expect(button(page, 'Open playlists')).toBeVisible();
});

test('the toolbar stays above the page and the sidebar, on every Music page', async ({
  page,
}) => {
  await page.goto('/#library/music?view=albums');
  await button(page, 'Open playlists').click();
  await expect(rowsOf(frame(page)).first()).toBeVisible();
  await expect(page.locator('[data-card-grid]')).toBeVisible();
  const toolbar = page.locator('[data-music-toolbar]');
  const scroller = page.locator('[data-music-scroll]');
  const lists = page.getByRole('group', { name: 'Music lists' });
  const toggle = button(page, 'Close playlists');
  const placed = (await toolbar.boundingBox())!;
  const toggled = (await toggle.boundingBox())!;
  const bottom = placed.y + placed.height;
  expect((await scroller.boundingBox())!.y).toBeCloseTo(bottom, 0);
  expect((await frame(page).boundingBox())!.y).toBeCloseTo(bottom, 0);
  // It lines up with the grid under it.
  const grid = (await page.locator('[data-card-grid]').boundingBox())!;
  expect(grid.x).toBeCloseTo(placed.x, 0);

  await scroller.evaluate((element) => (element.scrollTop = 300));
  await expect
    .poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  expect(await toolbar.boundingBox()).toEqual(placed);
  await expect(lists).toBeInViewport();

  // Opening a page swaps what it holds, without moving it or the toggle.
  await scroller.evaluate((element) => (element.scrollTop = 0));
  await page.locator('[data-card-grid] .card-primary').first().click();
  await expect(button(page, /^Back to/)).toBeVisible();
  await expect(lists).toHaveCount(0);
  expect(await toolbar.boundingBox()).toEqual(placed);
  expect(await toggle.boundingBox()).toEqual(toggled);
  expect((await frame(page).boundingBox())!.y).toBeCloseTo(bottom, 0);

  // The sidebar opens and closes under it.
  await toggle.click();
  await expect(button(page, 'Open playlists')).toBeVisible();
  expect(await toolbar.boundingBox()).toEqual(placed);
  expect(await button(page, 'Open playlists').boundingBox()).toEqual(toggled);
});

for (const width of [721, 800, 1000]) {
  test(`the sidebar stays where it is between pages in a ${width}px window`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#library/music');
    await button(page, 'Open playlists').click();
    await expect(rowsOf(frame(page)).first()).toBeVisible();
    await expect(button(page, 'Scan music library')).toBeInViewport();
    const placed = await frame(page).boundingBox();
    await page.locator('[data-card-grid] .card-primary').first().click();
    await expect(button(page, /^Back to/)).toBeVisible();
    expect(await frame(page).boundingBox()).toEqual(placed);
    await button(page, /^Back to/).click();
    await expect(button(page, 'Scan music library')).toBeVisible();
    expect(await frame(page).boundingBox()).toEqual(placed);
  });
}

for (const width of [721, 1000, 1100]) {
  test(`the sort menu fits a ${width}px window with the sidebar open`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#library/music');
    await button(page, 'Open playlists').click();
    await button(page, 'Sort artists').click();
    const menu = page.getByRole('dialog', { name: 'Sort artists' });
    const opened = (await menu.boundingBox())!;
    expect(opened.x).toBeGreaterThanOrEqual(0);
    expect(opened.x + opened.width).toBeLessThanOrEqual(width);
    await button(menu, 'Name descending').click();
    await expect(button(menu, 'Name descending')).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });
}

test('the toolbar stays small on a phone, with its menu under it', async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#library/music');
  const toolbar = page.locator('[data-music-toolbar]');
  const scroller = page.locator('[data-music-scroll]');
  await expect(page.locator('[data-card-grid]')).toBeVisible();
  const placed = (await toolbar.boundingBox())!;
  // Two rows: its lists and its toggle, then its search and buttons.
  expect(placed.height).toBeLessThan(90);
  await expect(button(page, 'Scan music library')).toBeInViewport();

  await scroller.evaluate((element) => (element.scrollTop = 300));
  await expect
    .poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  expect(await toolbar.boundingBox()).toEqual(placed);

  await button(page, 'Sort artists').click();
  const menu = page.getByRole('dialog', { name: 'Sort artists' });
  const opened = (await menu.boundingBox())!;
  expect(opened.y).toBeGreaterThanOrEqual(placed.y + placed.height - 1);
  expect(opened.x).toBeGreaterThanOrEqual(0);
  expect(opened.x + opened.width).toBeLessThanOrEqual(390);
});

const animatedGrid = '[data-card-grid]';
const motionTargets = {
  sidebar: '[data-youtube-watchlist-scale]',
  first: '[data-card-grid] > :nth-child(1)',
  second: '[data-card-grid] > :nth-child(2)',
};
const boxes = (page: Page) =>
  page.evaluate((targets) => {
    return Object.fromEntries(
      Object.entries(targets).map(([name, selector]) => {
        const box = document.querySelector(selector)!.getBoundingClientRect();
        return [name, [box.left, box.top, box.width, box.height]];
      }),
    );
  }, motionTargets);
// The boxes at the start, during and at the end of the animation an action starts.
// Seeking it makes this independent of how many frames the machine manages.
async function animationMoments(
  page: Page,
  animated: string,
  action: () => Promise<unknown>,
) {
  const moments = page.evaluate(
    ({ animated, targets }) =>
      new Promise<Record<string, number[]>[]>((resolve) => {
        const read = () =>
          Object.fromEntries(
            Object.entries(targets).map(([name, selector]) => {
              const box = document
                .querySelector(selector)!
                .getBoundingClientRect();
              return [name, [box.left, box.top, box.width, box.height]];
            }),
          );
        const wait = () => {
          const running = document.getAnimations();
          if (
            !running.some((animation) =>
              (animation.effect as KeyframeEffect | null)?.target?.matches(
                animated,
              ),
            )
          )
            return void requestAnimationFrame(wait);
          for (const animation of running) animation.pause();
          const at = (progress: number) => {
            for (const animation of running)
              animation.currentTime =
                Number(animation.effect!.getComputedTiming().duration) *
                progress;
            return read();
          };
          // The easing covers most of the distance early on.
          const seen = [at(0), at(0.1), at(0.999)];
          for (const animation of running) animation.finish();
          resolve(seen);
        };
        requestAnimationFrame(wait);
      }),
    { animated, targets: motionTargets },
  );
  await action();
  return moments;
}
const settled = async (page: Page) => {
  await page.waitForFunction(() => !document.getAnimations().length);
  return boxes(page);
};
const expectBoxes = (
  actual: Record<string, number[]>,
  expected: Record<string, number[]>,
  names: string[],
) => {
  for (const name of names)
    actual[name].forEach((value, index) =>
      expect(
        Math.abs(value - expected[name][index]),
        `${name} ${index}: ${value} is not ${expected[name][index]}`,
      ).toBeLessThan(1.5),
    );
};
const between = (value: number, one: number, other: number) =>
  value > Math.min(one, other) + 1 && value < Math.max(one, other) - 1;

for (const { title, route, name } of [
  { title: 'YouTube', route: '/#online/youtube', name: 'watchlists' },
  { title: 'Music', route: '/#library/music', name: 'playlists' },
]) {
  test(`${title}'s grid and sidebar animate as one, from where they are`, async ({
    page,
  }) => {
    const edge = page.viewportSize()!.width;
    await page.goto(route);
    await expect(page.locator(motionTargets.second)).toBeVisible();

    const closed = await settled(page);
    const opening = await animationMoments(page, animatedGrid, () =>
      button(page, `Open ${name}`).click(),
    );
    const open = await settled(page);
    expectBoxes(opening[0], closed, ['first', 'second']);
    expectBoxes(opening[2], open, ['first', 'second', 'sidebar']);
    // Both dimensions move together, and the sidebar slides in at its own size.
    expect(between(opening[1].first[2], closed.first[2], open.first[2])).toBe(
      true,
    );
    expect(between(opening[1].first[3], closed.first[3], open.first[3])).toBe(
      true,
    );
    expect(Math.abs(opening[0].sidebar[0] - edge)).toBeLessThan(1.5);
    expect(between(opening[1].sidebar[0], open.sidebar[0], edge)).toBe(true);
    for (const moment of opening)
      expect(Math.abs(moment.sidebar[2] - open.sidebar[2])).toBeLessThan(1.5);

    const closing = await animationMoments(page, animatedGrid, () =>
      button(page, `Close ${name}`).click(),
    );
    const again = await settled(page);
    expectBoxes(closing[0], open, ['first', 'second', 'sidebar']);
    expectBoxes(closing[2], again, ['first', 'second']);
    expect(between(closing[1].first[3], open.first[3], again.first[3])).toBe(
      true,
    );
    expect(closing[2].sidebar[0]).toBeGreaterThan(edge - 1.5);

    // The navigation rail moves the same grid.
    await button(page, `Open ${name}`).click();
    await expect(button(page, `Close ${name}`)).toBeVisible();
    const beside = await settled(page);
    const collapsing = await animationMoments(page, animatedGrid, () =>
      button(page, /collapse/i)
        .first()
        .click(),
    );
    const collapsed = await settled(page);
    expectBoxes(collapsing[0], beside, ['first', 'second', 'sidebar']);
    expectBoxes(collapsing[2], collapsed, ['first', 'second', 'sidebar']);
    expect(
      between(collapsing[1].first[3], beside.first[3], collapsed.first[3]),
    ).toBe(true);
  });

  test(`${title}'s open sidebar slides in on arriving`, async ({ page }) => {
    await page.goto(route);
    await button(page, `Open ${name}`).click();
    await expect(button(page, `Close ${name}`)).toBeVisible();
    const final = await settled(page);

    await page.getByRole('link', { name: 'Statistics', exact: true }).click();
    await expect(page.locator(motionTargets.sidebar)).toHaveCount(0);
    await page.waitForFunction(() => !document.getAnimations().length);

    const arriving = await animationMoments(page, '[data-page-slide]', () =>
      page.getByRole('link', { name: title, exact: true }).click(),
    );
    const width = final.sidebar[2];
    expect(
      Math.abs(arriving[0].sidebar[0] - final.sidebar[0] - width),
    ).toBeLessThan(1.5);
    expect(
      between(
        arriving[1].sidebar[0],
        final.sidebar[0],
        final.sidebar[0] + width,
      ),
    ).toBe(true);
    expect(Math.abs(arriving[2].sidebar[0] - final.sidebar[0])).toBeLessThan(
      1.5,
    );
    for (const moment of arriving)
      expect(Math.abs(moment.sidebar[2] - width)).toBeLessThan(1.5);
  });
}

test('moving between Music pages leaves the sidebar where it is', async ({
  page,
}) => {
  await page.goto('/#library/music');
  await button(page, 'Open playlists').click();
  await expect(button(page, 'Close playlists')).toBeVisible();
  await settled(page);
  const sidebar = frame(page);
  const before = (await sidebar.boundingBox())!;
  const slid = () =>
    page.evaluate(
      () =>
        new Promise<boolean>((resolve) => {
          let frames = 0;
          const watch = () => {
            if (
              document
                .getAnimations()
                .some((animation) =>
                  (animation.effect as KeyframeEffect | null)?.target?.matches(
                    '[data-page-slide]',
                  ),
                )
            )
              return resolve(true);
            if (++frames > 60) return resolve(false);
            requestAnimationFrame(watch);
          };
          requestAnimationFrame(watch);
        }),
    );
  const going = slid();
  await page.locator('[data-card-grid] .card-primary').first().click();
  await expect(button(page, /^Back to/)).toBeVisible();
  expect(await going).toBe(false);
  expect(await sidebar.boundingBox()).toEqual(before);

  const returning = slid();
  await button(page, /^Back to/).click();
  await expect(
    page.locator('[data-card-grid] .card-primary').first(),
  ).toBeVisible();
  expect(await returning).toBe(false);
  expect(await sidebar.boundingBox()).toEqual(before);
});

test('the scroll of a Music page comes back with Back', async ({
  page,
  request,
}) => {
  // Cards large enough for the mock library to overflow the viewport.
  await request.patch('/api/v1/me/appearance', {
    data: { poster_columns: 5 },
  });
  await page.goto('/#library/music');
  const scroller = page.locator('[data-music-scroll]');
  const card = page.locator('[data-card-grid] .card-primary').nth(10);
  await expect(card).toBeVisible();
  const saved = await scroller.evaluate((element) => {
    element.scrollTop = 240;
    return element.scrollTop;
  });
  expect(saved).toBeGreaterThan(0);
  await card.evaluate((element) => (element as HTMLElement).click());
  await expect(button(page, /^Back to/)).toBeVisible();
  await expect
    .poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBe(0);
  await button(page, /^Back to/).click();
  await expect
    .poll(() => scroller.evaluate((element) => element.scrollTop))
    .toBe(saved);
});

// What paints somewhere else on screen on the first frame of the animation an
// action starts, leaving out the cards, whose row gaps ease in on YouTube's grid too.
async function firstFrameMoves(page: Page, action: () => Promise<unknown>) {
  const moves = page.evaluate(
    () =>
      new Promise<string[]>((resolve) => {
        const hasText = (element: Element) =>
          [...element.childNodes].some(
            (node) =>
              node.nodeType === Node.TEXT_NODE && node.textContent?.trim(),
          );
        const hasBox = (element: Element) => {
          const style = getComputedStyle(element);
          return (
            ['IMG', 'svg'].includes(element.tagName) ||
            style.backgroundColor !== 'rgba(0, 0, 0, 0)' ||
            style.backgroundImage !== 'none' ||
            Number.parseFloat(style.borderTopWidth) > 0 ||
            Number.parseFloat(style.borderLeftWidth) > 0
          );
        };
        const before = new Map(
          [
            ...document.querySelectorAll(
              '[data-music-scroll] *, [data-music-toolbar] *',
            ),
          ]
            .filter((element) => {
              const box = element.getBoundingClientRect();
              return (
                box.top < innerHeight &&
                box.bottom > 0 &&
                !element.closest('[data-card-grid]') &&
                (hasBox(element) || hasText(element))
              );
            })
            .map((element) => [element, element.getBoundingClientRect()]),
        );
        const wait = () => {
          const running = document.getAnimations();
          if (
            !running.some((animation) =>
              (animation.effect as KeyframeEffect | null)?.target?.matches(
                '[data-youtube-watchlist-scale]',
              ),
            )
          )
            return void requestAnimationFrame(wait);
          for (const animation of running) {
            animation.pause();
            animation.currentTime = 0;
          }
          const moved = [...before].flatMap(([element, box]) => {
            if (!element.isConnected) return [];
            const now = element.getBoundingClientRect();
            const edges = [now.left - box.left, now.top - box.top];
            // Text sits at its start, whatever width its box has by now.
            if (hasBox(element))
              edges.push(now.width - box.width, now.height - box.height);
            return edges.some((edge) => Math.abs(edge) > 1.5)
              ? [`${element.tagName} ${element.className}: ${edges}`]
              : [];
          });
          for (const animation of running) animation.finish();
          resolve(moved);
        };
        requestAnimationFrame(wait);
      }),
  );
  await action();
  return moves;
}

test('Music pages stay where they are on the first frame of the sidebar animation', async ({
  page,
}) => {
  const still = () =>
    page.waitForFunction(() => !document.getAnimations().length);
  const toggle = async () => {
    await still();
    for (const action of ['Open', 'Close']) {
      // The pointer leaves the row it's over first, which hides that row's icons.
      const toggled = button(page, `${action} playlists`).first();
      await toggled.hover();
      const moves = await firstFrameMoves(page, () => toggled.click());
      expect(moves).toEqual([]);
      await still();
    }
  };
  const cards = page.locator('[data-card-grid] .card-primary');
  await page.goto('/#library/music?view=tracks');
  await expect(page.locator('[data-track]').first()).toBeVisible();
  await toggle();

  await page.goto('/#library/music');
  await cards.first().click();
  await expect(button(page, /^Back to/)).toBeVisible();
  await expect(cards.first()).toBeVisible();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
  await toggle();

  await cards.first().click();
  await expect(page.locator('[data-track]').first()).toBeVisible();
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
  await toggle();
});

test('rows show one title line and options sit on their own lines', async ({
  page,
}) => {
  await page.goto('/#library/music');
  await button(page, 'Open playlists').click();
  const sidebar = frame(page);
  const row = rowsOf(sidebar).first();
  await expect(row).toBeVisible();
  await expect(row.locator('p').first()).toHaveCSS('white-space', 'nowrap');
  const height = await row.evaluate(
    (element) =>
      element.getBoundingClientRect().height /
      Number.parseFloat(
        getComputedStyle(
          element.closest<HTMLElement>('[data-youtube-watchlist-scale]')!,
        ).zoom,
      ),
  );
  expect(height).toBeLessThan(60);

  // The hover buttons stay clear of the track's length, on the title's line.
  await row.hover();
  const length = (await row.getByText('4:00').boundingBox())!;
  const titleBox = (await row.locator('p').first().boundingBox())!;
  expect(length.y).toBeLessThan(titleBox.y + titleBox.height);
  for (const icon of await row.locator('button svg').all()) {
    const box = (await icon.boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(length.y + length.height / 2);
  }

  await button(sidebar, 'Playlist settings').click();
  const lines = await Promise.all(
    ['Public', 'Others can add', 'Others can remove and reorder'].map((label) =>
      sidebar.getByText(label, { exact: true }).boundingBox(),
    ),
  );
  expect(lines[0]!.y).toBeLessThan(lines[1]!.y - lines[0]!.height);
  expect(lines[1]!.y).toBeLessThan(lines[2]!.y - lines[1]!.height);
});

test('without playlists there is only the field to create one', async ({
  page,
}) => {
  await page.route('**/api/v1/playlists', (route) =>
    route.request().method() === 'GET'
      ? route.fulfill({ json: { items: [] } })
      : route.fallback(),
  );
  await page.goto('/#library/music');
  await button(page, 'Open playlists').click();
  const sidebar = frame(page);
  await expect(button(sidebar, 'Sort playlist')).toHaveCount(0);
  await expect(button(sidebar, 'Playlist settings')).toHaveCount(0);
  await expect(sidebar.getByRole('list')).toHaveCount(0);
  await picker(sidebar, 'Playlists').click();
  await expect(
    sidebar
      .getByRole('dialog', { name: 'Playlists' })
      .getByPlaceholder('New playlist'),
  ).toBeVisible();
});

test('links from the old Playlists page open the playlist in the sidebar', async ({
  page,
}) => {
  await page.goto('/#playlists/playlist-2');
  await expect(picker(frame(page), 'On the road')).toBeVisible();
  await expect(rowsOf(frame(page))).toHaveCount(10);
  await expect(button(page, 'Close playlists')).toBeVisible();
  await expect(
    page
      .locator('.page-header, [data-page-content]')
      .getByRole('group', { name: 'Music lists' }),
  ).toBeVisible();
});

test('owners name playlists uniquely and open them up one level at a time', async ({
  page,
  request,
}, info) => {
  await openPlaylists(page);
  const sidebar = frame(page);
  await picker(sidebar, 'Evening listening').click();
  const menu = sidebar.getByRole('dialog', { name: 'Playlists' });
  await menu.getByPlaceholder('New playlist').fill('evening LISTENING');
  await button(menu, 'Create playlist').click();
  await expect(menu.getByRole('alert')).toHaveText(
    'You already have a playlist with this name',
  );
  await menu.getByPlaceholder('New playlist').fill('Road trip');
  await button(menu, 'Create playlist').click();
  await expect(picker(sidebar, 'Road trip')).toBeVisible();
  await expect(rowsOf(sidebar)).toHaveCount(0);

  await button(sidebar, 'Playlist settings').click();
  const settings = sidebar.getByRole('dialog', { name: 'Playlist settings' });
  const name = settings.getByRole('textbox', { name: 'Name' });
  await name.fill('ON THE ROAD');
  await name.press('Enter');
  await expect(settings.getByRole('alert')).toHaveText(
    'You already have a playlist with this name',
  );
  await name.fill('Road trip');
  await name.press('Enter');
  await expect(settings.getByRole('alert')).toHaveCount(0);

  const open = settings.getByRole('switch', { name: 'Public', exact: true });
  const add = settings.getByRole('switch', { name: 'Others can add' });
  const edit = settings.getByRole('switch', {
    name: 'Others can remove and reorder',
  });
  const created = async () => {
    const { items } = await (await request.get('/api/v1/playlists')).json();
    return items.find((list: { name: string }) => list.name === 'Road trip')
      .access;
  };
  await expect(open).not.toBeChecked();
  await expect(add).toBeDisabled();
  await expect(edit).toBeDisabled();
  await settings.getByText('Public', { exact: true }).click();
  await expect.poll(created).toBe('read');
  await expect(add).toBeEnabled();
  await expect(edit).toBeDisabled();
  await settings.getByText('Others can add').click();
  await expect.poll(created).toBe('add');
  await settings.getByText('Others can remove and reorder').click();
  await expect.poll(created).toBe('edit');
  await page.screenshot({ path: info.outputPath('settings.png') });

  await settings.getByText('Public', { exact: true }).click();
  await expect.poll(created).toBe('private');
  await expect(add).not.toBeChecked();
  await expect(edit).not.toBeChecked();
  await expect(add).toBeDisabled();
  await expect(edit).toBeDisabled();
});

test('rights decide what a playlist of someone else offers', async ({
  page,
  request,
}, info) => {
  await request.patch('/api/v1/playlists/playlist-1', {
    data: { access: 'edit' },
  });
  await request.post('/__mock/viewer', { data: { role: 'user' } });
  await openPlaylists(page);
  const sidebar = frame(page);
  await picker(sidebar, 'Evening listening').click();
  const menu = sidebar.getByRole('dialog', { name: 'Playlists' });
  await expect(menu.getByText('Robin', { exact: true })).toBeVisible();
  await expect(menu.getByRole('button', { name: /^On the road/ })).toHaveCount(
    0,
  );
  await expect(button(menu, /^Delete /)).toHaveCount(0);
  await page.screenshot({ path: info.outputPath('picker.png') });
  await menu.getByRole('button', { name: /^Shared mix/ }).click();

  // Add only: no sort, settings, removal or dragging.
  const rows = rowsOf(sidebar);
  await expect(rows).toHaveCount(5);
  await expect(button(sidebar, 'Sort playlist')).toHaveCount(0);
  await expect(button(sidebar, 'Playlist settings')).toHaveCount(0);
  await rows.first().hover();
  await expect(button(rows.first(), /^Actions for /)).toBeVisible();
  await expect(button(rows.first(), /from the playlist$/)).toHaveCount(0);
  const before = await keys(rows);
  const source = (await rows.first().boundingBox())!;
  await page.mouse.move(source.x + 30, source.y + 20);
  await page.mouse.down();
  await page.mouse.move(source.x + 30, source.y + 170, { steps: 6 });
  await page.mouse.up();
  expect(await keys(rows)).toEqual(before);

  // The edit level opens all of that but settings.
  await choose(page, 'Shared mix', 'Evening listening');
  await expect(rows).toHaveCount(12);
  await expect(button(sidebar, 'Sort playlist')).toBeVisible();
  await expect(button(sidebar, 'Playlist settings')).toHaveCount(0);
  await rows.first().hover();
  await button(rows.first(), /from the playlist$/).click();
  await expect(rows).toHaveCount(11);

  // Made private by its owner, it leaves the picker and the sidebar moves on.
  const elsewhere = (await keys(rows))[0];
  await request.post('/__mock/viewer', { data: { role: 'admin' } });
  await request.patch('/api/v1/playlists/playlist-1', {
    data: { access: 'private' },
  });
  await request.post('/__mock/viewer', { data: { role: 'user' } });
  await request.post('/api/v1/playlists/playlist-3/items', {
    data: { items: [elsewhere] },
  });
  await expect(picker(sidebar, 'Shared mix')).toBeVisible();
  await expect(rows).toHaveCount(6);
});

test('tracks, albums and artists add to a playlist once', async ({
  page,
  request,
}, info) => {
  await page.goto('/#library/music?view=tracks');
  const { items } = await playlist(request, 'playlist-1');
  const first = items[0];
  await button(page, `Actions for ${first.title}`).first().click();
  await page.getByRole('menuitem', { name: 'Add to playlist' }).click();
  await expect(
    page.getByRole('menuitem', { name: 'In Evening listening' }),
  ).toHaveAttribute('data-disabled', '');
  await page.screenshot({ path: info.outputPath('track-menu.png') });
  await page.getByRole('menuitem', { name: 'On the road' }).click();
  await expect(page.getByText(`Added ${first.title}.`)).toBeVisible();
  expect(
    (await playlist(request, 'playlist-2')).items.map((item) => item.id),
  ).toContain(first.id);
  await expect
    .poll(
      async () =>
        (await playlist(request, 'playlist-2')).items.filter(
          (item) => item.id === first.id,
        ).length,
    )
    .toBe(1);

  await page.goto('/#library/music/artist-0');
  await button(page, 'Add to playlist').click();
  await page.getByRole('menuitem', { name: 'Evening listening' }).click();
  await expect(page.getByText(/^Added \d+ tracks?\.$/)).toBeVisible();
  const added = (await playlist(request, 'playlist-1')).items.length;
  await page.reload();
  await button(page, 'Add to playlist').click();
  await page.getByRole('menuitem', { name: 'Evening listening' }).click();
  await expect(page.getByText('Added 0 tracks.')).toBeVisible();
  expect((await playlist(request, 'playlist-1')).items).toHaveLength(added);
});

test('manual order drags, and a sort is shared and stops dragging', async ({
  page,
  request,
}, info) => {
  await openPlaylists(page);
  const sidebar = frame(page);
  const rows = rowsOf(sidebar);
  const initial = await keys(rows);
  const source = (await rows.first().boundingBox())!,
    target = (await rows.nth(2).boundingBox())!;
  await page.mouse.move(source.x + 30, source.y + source.height / 2);
  await page.mouse.down();
  await page.mouse.move(source.x + 30, source.y + source.height / 2 + 10);
  await page.mouse.move(target.x + 30, target.y + target.height - 5, {
    steps: 6,
  });
  await page.mouse.up();
  const moved = [initial[1], initial[2], initial[0], ...initial.slice(3)];
  await expect.poll(() => keys(rows)).toEqual(moved);
  await expect
    .poll(async () =>
      (await playlist(request, 'playlist-1')).items.map((item) => item.id),
    )
    .toEqual(moved);

  await button(sidebar, 'Sort playlist').click();
  await button(sidebar, 'Title descending').click();
  await page.keyboard.press('Escape');
  await expect
    .poll(async () => (await playlist(request, 'playlist-1')).sort_mode)
    .toBe('title');
  const titles = (await playlist(request, 'playlist-1')).items
    .map((item) => item.title)
    .sort((a, b) =>
      b.localeCompare(a, undefined, { sensitivity: 'base', numeric: true }),
    );
  await expect(rows.locator('[role="button"]').first()).toHaveAccessibleName(
    titles[0],
  );
  await page.screenshot({ path: info.outputPath('sorted.png') });

  const sorted = await keys(rows);
  const start = (await rows.first().boundingBox())!;
  await page.mouse.move(start.x + 30, start.y + 20);
  await page.mouse.down();
  await page.mouse.move(start.x + 30, start.y + 170, { steps: 6 });
  await page.mouse.up();
  expect(await keys(rows)).toEqual(sorted);

  await page.reload();
  await expect.poll(() => keys(rowsOf(frame(page)))).toEqual(sorted);
});

test('a list of thousands renders and loads only what is near the viewport', async ({
  page,
}, info) => {
  const count = 5000;
  const tracks = Array.from({ length: count }, (_, index) => ({
    id: `big-${index}`,
    kind: 'track',
    title: `Track number ${index}`,
    artist: `Artist ${index % 40}`,
    album: 'Long album',
    year: 2020,
    available: true,
    metadata: {},
    duration: 180,
    added_at: index,
    artwork_url: `/api/v1/big-art/${index}.png`,
  }));
  const big = {
    id: 'big',
    name: 'A very long list',
    owner_id: 'user-admin',
    owner: 'admin',
    revision: 1,
    count,
    access: 'private',
    sort_mode: 'manual',
    sort_direction: 'asc',
    is_owner: true,
    can_add: true,
    can_edit: true,
  };
  const art = new Set<string>();
  await page.route('**/api/v1/big-art/**', (route) => {
    art.add(new URL(route.request().url()).pathname);
    return route.fulfill({
      status: 200,
      contentType: 'image/svg+xml',
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>',
    });
  });
  await page.route('**/api/v1/playlists', (route) =>
    route.fulfill({ json: { items: [big] } }),
  );
  await page.route('**/api/v1/playlists/big', (route) =>
    route.fulfill({ json: { ...big, items: tracks } }),
  );
  await page.goto('/#library/music');
  await button(page, 'Open playlists').click();
  const sidebar = frame(page);
  const rows = rowsOf(sidebar);
  await expect(rows.first()).toBeVisible();

  const scale = await sidebar
    .locator('[data-youtube-watchlist-scale]')
    .evaluate((element) => Number((element as HTMLElement).style.zoom));
  expect(scale).not.toBe(1);
  const list = sidebar.getByRole('list');
  const rendered = async () => ({
    rows: await rows.count(),
    images: await sidebar.locator('img').count(),
  });
  const first = await rendered();
  expect(first.rows).toBeLessThan(60);
  expect(first.images).toBeLessThan(60);
  await expect.poll(() => art.size).toBeGreaterThan(0);
  expect(art.size).toBeLessThan(60);

  // Rows fill the viewport wherever it is, zoomed or not.
  for (const fraction of [0.25, 0.5, 0.75, 1]) {
    const expected = await list.evaluate((element, part) => {
      element.scrollTop = (element.scrollHeight - element.clientHeight) * part;
      return Math.floor(element.scrollTop / 53);
    }, fraction);
    await expect(
      list.locator(`[data-list-item-key="big-${expected}"]`),
    ).toBeVisible();
    const covered = await list.evaluate((element) => {
      const box = element.getBoundingClientRect();
      return [0.1, 0.5, 0.9].map(
        (at) =>
          !!document
            .elementFromPoint(box.left + 40, box.top + box.height * at)
            ?.closest('[data-list-item-key]'),
      );
    });
    expect(covered).toEqual([true, true, true]);
    expect((await rendered()).rows).toBeLessThan(60);
  }
  await expect(
    list.locator(`[data-list-item-key="big-${count - 1}"]`),
  ).toBeInViewport();
  await page.screenshot({ path: info.outputPath('end-of-list.png') });
  expect(art.size).toBeLessThan(300);

  // Holding a dragged row at the edge reaches rows far from it.
  const total = await list.evaluate((element) => {
    element.scrollTop = 0;
    return element.scrollHeight;
  });
  expect(total).toBeGreaterThan(count * 53 * 0.99);
  await expect(list.locator('[data-list-item-key="big-0"]')).toBeVisible();
  const box = (await list.boundingBox())!;
  const source = (await rows.first().boundingBox())!;
  await page.mouse.move(source.x + 30, source.y + 15);
  await page.mouse.down();
  await page.mouse.move(source.x + 30, source.y + 30, { steps: 3 });
  await page.mouse.move(source.x + 30, box.y + box.height - 4, { steps: 6 });
  await expect
    .poll(() => list.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(1000);
  await page.mouse.up();
});

test('the dock lists a queue of thousands by what is near the viewport', async ({
  page,
  request,
}) => {
  const real = (await playlist(request, 'playlist-1')).items;
  const count = 5000;
  const big = {
    ...(await (await request.get('/api/v1/playlists/playlist-1')).json()),
    count,
    items: [
      ...real,
      ...Array.from({ length: count - real.length }, (_, index) => ({
        ...real[0],
        id: `big-${index}`,
        title: `Track number ${index}`,
      })),
    ],
  };
  await page.route('**/api/v1/playlists/playlist-1', (route) =>
    route.fulfill({ json: big }),
  );
  await openPlaylists(page);
  await rowsOf(frame(page)).first().locator('[role="button"]').click();
  const dock = page.getByRole('region', { name: 'Music player' });
  await expect(dock.getByText(`1 of ${count}`, { exact: true })).toBeVisible();
  await button(dock, 'Queue').click();
  const queue = page.locator('[data-popover-content]');
  const entries = queue.locator('li');
  await expect(entries.first()).toBeVisible();
  expect(await entries.count()).toBeLessThan(40);

  await queue.locator('[data-queue-scroller]').evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(
    button(queue, `Play Track number ${count - real.length - 1}`),
  ).toBeInViewport();
  expect(await entries.count()).toBeLessThan(40);
});

test('phones open playlists in a drawer above the dock, kept open while playing', async ({
  browser,
}, info) => {
  const context = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await page.goto('/#library/music');
  await button(page, 'Open playlists').tap();
  const drawer = page.getByRole('dialog', { name: 'Music playlists' });
  await expect(drawer).toBeVisible();
  const rows = rowsOf(drawer);
  await expect(rows).toHaveCount(12);
  await rows.first().locator('[role="button"]').tap();
  const dock = page.getByRole('region', { name: 'Music player' });
  await expect(dock).toBeVisible();
  await expect(drawer).toBeVisible();
  await expect
    .poll(async () => {
      const panel = (await drawer.boundingBox())!;
      const bar = (await dock.boundingBox())!;
      return Math.abs(panel.y + panel.height - bar.y);
    })
    .toBeLessThan(2);
  await page.screenshot({ path: info.outputPath('phone-drawer.png') });

  await hold(page, rows.nth(1).locator('[role="button"]'), async () => {
    await expect(
      page.getByRole('dialog', { name: /^Actions for / }),
    ).toBeVisible();
  });
  const sheet = page.getByRole('dialog', { name: /^Actions for / });
  for (const name of [
    'Play next',
    'Add to queue',
    'Add to playlist',
    'Details',
    'Remove from playlist',
  ])
    await expect(sheet.getByRole('button', { name })).toBeVisible();
  await page.screenshot({ path: info.outputPath('phone-sheet.png') });
  await sheet.getByRole('button', { name: 'Remove from playlist' }).tap();
  await expect(rows).toHaveCount(11);
  await context.close();
});

test('a queue reopened with the app keeps following its playlist until cleared', async ({
  page,
  request,
}) => {
  await openPlaylists(page);
  const rows = rowsOf(frame(page));
  const tracks = (await playlist(request, 'playlist-1')).items;
  await rows.nth(1).locator('[role="button"]').click();
  const dock = page.getByRole('region', { name: 'Music player' });
  await expect(dock.getByText('2 of 12', { exact: true })).toBeVisible();
  await button(dock, 'Pause').click();
  await expect(button(dock, 'Play')).toBeVisible();

  await page.reload();
  await expect(dock.getByText('2 of 12', { exact: true })).toBeVisible();
  await expect(rows).toHaveCount(12);
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[5].id}`);
  await expect(dock.getByText('2 of 11', { exact: true })).toBeVisible();

  const queue = page.locator('[data-popover-content]');
  const entries = queue.getByRole('button', { name: /^Play / });
  await button(dock, 'Queue').click();
  await button(queue, 'Clear').click();
  await expect(entries).toHaveCount(1);
  await page.reload();
  await expect(rows).toHaveCount(11);
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[6].id}`);
  await expect(rows).toHaveCount(10);
  await button(dock, 'Queue').click();
  await expect(entries).toHaveCount(1);
});

test('a reopened queue catches up with its playlist, and keeps hand edits to an unchanged one', async ({
  page,
  request,
}) => {
  await openPlaylists(page);
  const rows = rowsOf(frame(page));
  const tracks = (await playlist(request, 'playlist-1')).items;
  await rows.nth(1).locator('[role="button"]').click();
  const dock = page.getByRole('region', { name: 'Music player' });
  const place = (text: string) => dock.getByText(text, { exact: true });
  await expect(place('2 of 12')).toBeVisible();
  await button(dock, 'Pause').click();
  await expect(button(dock, 'Play')).toBeVisible();
  await button(dock, 'Queue').click();
  await page
    .locator('[data-popover-content] li')
    .nth(5)
    .getByRole('button', { name: /^Remove / })
    .click();
  await expect(place('2 of 11')).toBeVisible();

  await page.reload();
  await expect(rows).toHaveCount(12);
  await expect(place('2 of 11')).toBeVisible();

  await page.goto('about:blank');
  for (const track of [tracks[6], tracks[7]])
    await request.delete(`/api/v1/playlists/playlist-1/items/${track.id}`);
  await page.goto('/#library/music');
  await expect(rows).toHaveCount(10);
  await expect(place('2 of 10')).toBeVisible();
});

test('a track removed while playing leaves the queue once it stops', async ({
  page,
  request,
}) => {
  await openPlaylists(page);
  const rows = rowsOf(frame(page));
  const tracks = (await playlist(request, 'playlist-1')).items;
  await rows.nth(1).locator('[role="button"]').click();
  const dock = page.getByRole('region', { name: 'Music player' });
  const place = (text: string) => dock.getByText(text, { exact: true });
  await expect(place('2 of 12')).toBeVisible();
  await button(dock, 'Repeat off').click();

  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[1].id}`);
  await expect(rows).toHaveCount(11);
  await expect(place('2 of 12')).toBeVisible();
  await expect(dock).toContainText(tracks[1].title);

  // Coming to its end.
  const seek = dock.getByLabel('Music position');
  await expect(seek).not.toHaveAttribute('max', '0');
  await seek.fill(String(Number(await seek.getAttribute('max')) - 0.5));
  await expect(place('2 of 11')).toBeVisible();
  await expect(dock).toContainText(tracks[2].title);

  // Skipped, once it has started and can't reach its end first.
  await expect
    .poll(() => seek.evaluate((input: HTMLInputElement) => Number(input.value)))
    .toBeGreaterThan(0);
  await button(dock, 'Pause').click();
  await expect(button(dock, 'Play')).toBeVisible();
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[2].id}`);
  await expect(rows).toHaveCount(10);
  await button(dock, 'Next track').click();
  await expect(place('2 of 10')).toBeVisible();
  await expect(dock).toContainText(tracks[3].title);
});

test('a removed track reopened with the app still leaves once it stops', async ({
  page,
  request,
}) => {
  await openPlaylists(page);
  const rows = rowsOf(frame(page));
  const tracks = (await playlist(request, 'playlist-1')).items;
  await rows.nth(1).locator('[role="button"]').click();
  const dock = page.getByRole('region', { name: 'Music player' });
  const place = (text: string) => dock.getByText(text, { exact: true });
  await expect(place('2 of 12')).toBeVisible();
  await button(dock, 'Pause').click();
  await expect(button(dock, 'Play')).toBeVisible();

  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[1].id}`);
  await expect(rows).toHaveCount(11);
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.entries(localStorage).some(
          ([key, value]) => key.endsWith(':music') && JSON.parse(value).leaving,
        ),
      ),
    )
    .toBe(true);

  await page.reload();
  await expect(rows).toHaveCount(11);
  await expect(place('2 of 12')).toBeVisible();
  await button(dock, 'Next track').click();
  await expect(place('2 of 11')).toBeVisible();
  await expect(dock).toContainText(tracks[2].title);
});

test('an add that took nothing still shows what someone else added meanwhile', async ({
  page,
  request,
}) => {
  const items = '/api/v1/playlists/playlist-1/items';
  const tracks = (await playlist(request, 'playlist-1')).items;
  const outside = tracks[11];
  await request.delete(`${items}/${outside.id}`);
  await openPlaylists(page);
  const rows = rowsOf(frame(page));
  await expect(rows).toHaveCount(11);
  await page.goto(`/#library/music/${tracks[0].parent_id}`);
  await expect(page.locator('[data-track]').first()).toBeVisible();

  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route(`**${items}`, async (route) => {
    await held;
    await route.continue();
  });
  await button(page, 'Add to playlist').click();
  await page.getByRole('menuitem', { name: 'Evening listening' }).click();
  await request.post(items, { data: { items: [outside.id] } });
  release();
  await expect(page.getByText('Added 0 tracks.')).toBeVisible();
  await expect(rows).toHaveCount(12);
});

test('the dock follows its playlist over hand edits until it is cleared', async ({
  page,
  request,
}, info) => {
  await openPlaylists(page);
  const sidebar = frame(page);
  const rows = rowsOf(sidebar);
  const tracks = (await playlist(request, 'playlist-1')).items;
  await rows.nth(1).locator('[role="button"]').click();
  await expect(rows.nth(1).locator('[data-playing="true"]')).toBeVisible();
  const left = (await rows
    .nth(1)
    .getByText(/^-\d+:\d\d$/)
    .boundingBox())!;
  const title = (await rows.nth(1).locator('p').first().boundingBox())!;
  expect(left.y).toBeLessThan(title.y + title.height);
  const dock = page.getByRole('region', { name: 'Music player' });
  await expect(dock).toContainText(tracks[1].title);
  await expect(dock.getByText('2 of 12', { exact: true })).toBeVisible();

  await button(dock, 'Queue').click();
  const queue = page.locator('[data-popover-content]');
  const entries = queue.getByRole('button', { name: /^Play / });
  await expect(entries).toHaveCount(12);
  await page.screenshot({ path: info.outputPath('queue.png') });

  const queued = () =>
    entries.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute('aria-label')),
    );
  const plays = (list: typeof tracks) =>
    list.map((track) => `Play ${track.title}`);

  // Hand edits last until the playlist changes.
  await queue
    .locator('li')
    .nth(5)
    .getByRole('button', { name: /^Remove / })
    .click();
  await expect(entries).toHaveCount(11);
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[3].id}`);
  const remaining = tracks.filter((_, index) => index !== 3);
  await expect.poll(queued).toEqual(plays(remaining));

  // The playing track outlasts its removal, and keeps its place.
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[1].id}`);
  await expect(rows).toHaveCount(10);
  await expect(dock).toContainText(tracks[1].title);
  await expect.poll(queued).toEqual(plays(remaining));

  // Order follows too.
  await request.post(
    `/api/v1/playlists/playlist-1/items/${tracks[0].id}/move`,
    { data: { before: null } },
  );
  await expect
    .poll(queued)
    .toEqual(plays([...remaining.slice(1), remaining[0]]));

  await button(queue, 'Clear').click();
  await expect(entries).toHaveCount(1);
  await request.delete(`/api/v1/playlists/playlist-1/items/${tracks[2].id}`);
  await expect(rows).toHaveCount(9);
  await expect(entries).toHaveCount(1);
});
