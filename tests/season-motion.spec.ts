import { expect, test, type Locator, type Page } from '@playwright/test';
import {
  finishAnimations,
  pauseAnimations,
  seekAnimations,
} from './helpers/paused-animations';
import { responseGate } from './helpers/response-gate';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

test.afterEach(async ({ request }) => {
  expect(
    (await (await request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
});

// Tall enough to hold every row. One that starts past the bottom of the screen
// fades in instead of moving, and where that is depends on the system font.
test.use({ viewport: { width: 1440, height: 1600 } });

const tab = (page: Page, name: string) =>
  page.getByRole('tab', { name, exact: false });
const episodes = (page: Page, season: number) =>
  page.getByRole('list', { name: `Season ${season} episodes` });
const listed = (page: Page) => page.getByRole('tabpanel').getByRole('listitem');
const filter = (page: Page, name: RegExp) =>
  page.getByRole('group', { name: 'Show episodes' }).getByRole('button', {
    name,
  });
const watched = (page: Page, title: string) =>
  page.getByRole('button', { name: `Watched: ${title}`, exact: true });
// Copies in a layer carry no motion keys, so this only finds the row itself.
const more = (page: Page, title: string) =>
  page
    .getByRole('tabpanel')
    .getByRole('button', { name: `More actions for ${title}`, exact: true });
const rows = (page: Page) =>
  page.locator('[role="tabpanel"] ol [data-episode]');
const row = (page: Page, season: number, number: number) =>
  page.locator(
    `[role="tabpanel"] ol [data-motion-row^="${season}:${number}:"]`,
  );

function motion(root: Locator) {
  return {
    root,
    content: root.locator(':scope > :not([data-motion-layer])'),
    layer: root.locator(':scope > [data-motion-layer]'),
    copies: root.locator(':scope > [data-motion-layer] > *'),
  };
}
const panel = (page: Page) =>
  motion(page.locator('[role="tabpanel"] > [aria-busy]'));
const list = (page: Page) => motion(panel(page).content.locator('[aria-busy]'));

// The keyframes of the animation changing this property, if one is.
const frames = (
  element: Locator,
  property: 'opacity' | 'height' | 'transform',
) =>
  element.evaluate((node, property) => {
    for (const animation of node.getAnimations()) {
      const keyframes = (animation.effect as KeyframeEffect).getKeyframes();
      if (property in keyframes[0])
        return keyframes.map((frame) => String(frame[property]));
    }
    return null;
  }, property);
const paint = (element: Locator) =>
  element.evaluate((node) => {
    for (const animation of node.getAnimations()) {
      const [first] = (animation.effect as KeyframeEffect).getKeyframes();
      if ('transform' in first) return String(first.backgroundColor);
    }
    return null;
  });
const opacity = (element: Locator) =>
  element.evaluate((node) => Number(getComputedStyle(node).opacity));
const background = (element: Locator) =>
  element.evaluate((node) => getComputedStyle(node).backgroundColor);
const heldStyle = (element: Locator) =>
  element.evaluate((node: HTMLElement) => [
    node.style.height,
    node.style.overflow,
  ]);

const watchLayers = (page: Page) =>
  page.addInitScript(() => {
    new MutationObserver(() => {
      if (document.querySelector('[data-motion-layer]'))
        document.documentElement.dataset.layerSeen = 'true';
    }).observe(document, { childList: true, subtree: true });
  });
const layerSeen = (page: Page) =>
  page.evaluate(() => document.documentElement.dataset.layerSeen);
const nextFrame = (page: Page) =>
  page.evaluate(() => new Promise(requestAnimationFrame));

async function open(page: Page) {
  await page.goto('/#library/shows/show-0');
  await expect(episodes(page, 1)).toBeVisible();
}

test('a season tab crossfades to the other season', async ({ page }) => {
  await open(page);
  await pauseAnimations(page);
  await tab(page, 'Season 2').click();
  const { root, content, layer, copies } = panel(page);
  await expect(episodes(page, 2)).toBeVisible();
  await expect(listed(page)).toHaveCount(6);
  await expect(
    copies.locator('[aria-label="Season 1 episodes"]'),
  ).toBeAttached();
  await expect.poll(() => frames(content, 'opacity')).toEqual(['0', '1']);
  expect(await frames(copies, 'opacity')).toEqual(['1', '0']);
  const height = await frames(root, 'height');
  expect(parseFloat(height![1])).toBeGreaterThan(parseFloat(height![0]));
  await finishAnimations(page);
  await expect(layer).toHaveCount(0);
  expect(await heldStyle(root)).toEqual(['', '']);
});

test('arrow keys crossfade and keep focus on the tabs', async ({ page }) => {
  await open(page);
  await pauseAnimations(page);
  await tab(page, 'Season 1').focus();
  await page.keyboard.press('ArrowRight');
  await expect(tab(page, 'Season 2')).toBeFocused();
  await expect(tab(page, 'Season 2')).toHaveAttribute('aria-selected', 'true');
  await expect(
    panel(page).copies.locator('[aria-label="Season 1 episodes"]'),
  ).toBeAttached();
  await page.keyboard.press('Home');
  await expect(tab(page, 'Season 1')).toBeFocused();
  await expect(episodes(page, 1)).toBeVisible();
});

test('the selected tab starts nothing when clicked again', async ({ page }) => {
  await watchLayers(page);
  await open(page);
  await tab(page, 'Season 1').click();
  await nextFrame(page);
  expect(await layerSeen(page)).toBeUndefined();
});

test('an interrupted crossfade carries on from what is on screen', async ({
  page,
}) => {
  await open(page);
  await pauseAnimations(page);
  await tab(page, 'Season 2').click();
  const { content, copies } = panel(page);
  await expect(copies).toHaveCount(1);
  await seekAnimations(page, 0.5);
  const season2 = await opacity(content);
  const season1 = await opacity(copies);
  expect(season2).toBeGreaterThan(0);
  expect(season2).toBeLessThan(1);

  await tab(page, 'Season 1').click();
  await expect(copies).toHaveCount(2);
  await expect(episodes(page, 1)).toBeVisible();
  expect(await opacity(copies.first())).toBeCloseTo(season1);
  await expect(
    copies.last().locator('[aria-label="Season 2 episodes"]'),
  ).toBeAttached();
  const fade = await frames(copies.last(), 'opacity');
  expect(Number(fade![0])).toBeCloseTo(season2);
  await expect.poll(() => frames(content, 'opacity')).toEqual(['0', '1']);

  // Its fade ends first, and that must not bring it back.
  await copies.first().evaluate((copy) => copy.getAnimations()[0].finish());
  expect(await opacity(copies.first())).toBe(0);

  await finishAnimations(page);
  await expect(panel(page).layer).toHaveCount(0);
  expect(await opacity(content)).toBe(1);
});

test('the episode filter glides the rows that stay', async ({ page }) => {
  await open(page);
  await watched(page, 'First Contact').click();
  // The next episode moves on to the second one once the seasons reload.
  await expect(row(page, 1, 2)).toHaveClass(/bg-accent-soft/);
  await pauseAnimations(page);
  await filter(page, /^Unwatched 3/).click();
  const { root, layer, copies } = list(page);
  await expect(row(page, 1, 1)).toHaveCount(0);
  await expect(copies).toHaveCount(1);
  await expect(copies).toContainText('First Contact');
  await expect.poll(() => frames(row(page, 1, 2), 'transform')).not.toBeNull();
  await expect.poll(() => frames(row(page, 1, 3), 'transform')).not.toBeNull();
  // The next episode stays highlighted while it moves.
  expect(await paint(row(page, 1, 2))).not.toBe(await paint(row(page, 1, 3)));
  await finishAnimations(page);
  await expect(layer).toHaveCount(0);
  await expect(listed(page)).toHaveCount(3);
  expect(await heldStyle(root)).toEqual(['', '']);

  await filter(page, /^All 4/).click();
  await expect(listed(page)).toHaveCount(4);
  await expect.poll(() => frames(row(page, 1, 1), 'opacity')).not.toBeNull();
  await expect.poll(() => frames(row(page, 1, 2), 'transform')).not.toBeNull();
});

test('an episode marked watched leaves the Unwatched filter gradually', async ({
  page,
}) => {
  await open(page);
  await filter(page, /^Unwatched 4/).click();
  await pauseAnimations(page);
  await watched(page, 'First Contact').click();
  const { copies, layer } = list(page);
  await expect(row(page, 1, 1)).toHaveCount(0);
  await expect(copies).toContainText('First Contact');
  await expect.poll(() => frames(row(page, 1, 2), 'transform')).not.toBeNull();
  // The reload that follows every mark brings the same rows, and must not end it.
  await expect(row(page, 1, 2)).toHaveClass(/bg-accent-soft/);
  await expect(copies).toHaveCount(1);
  // It made the second episode the next one, which shows before the row lands.
  await seekAnimations(page, 0.9);
  expect(await background(row(page, 1, 2))).not.toBe(
    await background(row(page, 1, 3)),
  );
  await finishAnimations(page);
  await expect(layer).toHaveCount(0);
  await expect(listed(page)).toHaveCount(3);
});

test('a season marked watched empties the Unwatched filter gradually', async ({
  page,
}) => {
  await open(page);
  await filter(page, /^Unwatched 4/).click();
  await pauseAnimations(page);
  await page.getByRole('button', { name: 'Mark season watched' }).click();
  const { root, copies, layer } = list(page);
  await expect(page.getByText('No episode matches.')).toBeVisible();
  // Rows past the bottom of the screen go without being seen.
  await expect(copies.first()).toContainText('First Contact');
  const height = await frames(root, 'height');
  expect(parseFloat(height![1])).toBeLessThan(parseFloat(height![0]));
  await finishAnimations(page);
  await expect(layer).toHaveCount(0);
  expect(await heldStyle(root)).toEqual(['', '']);
});

for (const { owner, item, left } of [
  { owner: 'First Contact', item: 'Mark watched', left: 3 },
  { owner: 'The Signal', item: 'Mark watched up to here', left: 1 },
  { owner: 'Season 1', item: 'Mark season watched', left: 0 },
]) {
  test(`${item} in a menu leaves the Unwatched filter gradually`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await open(page);
    await filter(page, /^Unwatched 4/).click();
    await more(page, owner).click();
    const reload = page.waitForResponse('**/seasons');
    await pauseAnimations(page);
    await page.getByRole('menuitem', { name: item, exact: true }).click();
    const { copies, layer } = list(page);
    await expect(row(page, 1, 1)).toHaveCount(0);
    await expect(copies.first()).toContainText('First Contact');
    await expect(rows(page)).toHaveCount(left);
    // Still reloads, though the row whose menu asked for it may be gone by now.
    await reload;
    await finishAnimations(page);
    await expect(layer).toHaveCount(0);
    await expect(rows(page)).toHaveCount(left);
    await expect(tab(page, 'Season 1')).toHaveAttribute(
      'title',
      new RegExp(`${4 - left} watched`),
    );
    expect(errors).toEqual([]);
  });
}

test('a refresh that changes the height of the list stops holding it', async ({
  page,
}) => {
  await open(page);
  await watched(page, 'First Contact').click();
  await expect(filter(page, /^Unwatched 3/)).toBeVisible();
  const gate = responseGate();
  await page.route('**/seasons', async (route) => {
    const response = await route.fetch();
    const json = await response.json();
    const [season] = json.seasons;
    for (const number of [5, 6])
      season.episodes.push({
        ...season.episodes[1],
        id: `new-${number}`,
        number,
        title: `New episode ${number}`,
      });
    await gate.promise;
    await route.fulfill({ response, json });
  });
  await more(page, 'Season 1').click();
  await page
    .getByRole('menuitem', { name: 'Add to Watch Later', exact: true })
    .click();
  await expect(page.getByRole('menu')).toHaveCount(0);

  await pauseAnimations(page);
  await filter(page, /^Unwatched 3/).click();
  const { root, layer, copies } = list(page);
  await expect(copies).toHaveCount(1);
  await expect(root).toHaveCSS('overflow', 'clip');
  gate.release();
  await expect(rows(page)).toHaveCount(5);
  await expect.poll(() => heldStyle(root)).toEqual(['', '']);
  expect(await frames(root, 'height')).toBeNull();
  // What is fading and moving has no reason to stop.
  await expect(copies).toHaveCount(1);
  await finishAnimations(page);
  await expect(layer).toHaveCount(0);
  expect(await heldStyle(root)).toEqual(['', '']);
});

test('a filter chosen during a crossfade leaves it running', async ({
  page,
}) => {
  await open(page);
  await pauseAnimations(page);
  await tab(page, 'Season 2').click();
  const { root, content, copies } = panel(page);
  await expect(copies).toHaveCount(1);
  await seekAnimations(page, 0.5);
  const incoming = await opacity(content);
  const outgoing = await opacity(copies);

  await filter(page, /^Missing 1/).click();
  await expect(listed(page)).toHaveCount(1);
  await expect.poll(() => frames(list(page).root, 'height')).not.toBeNull();
  await seekAnimations(page, 0.5);
  // It now follows the list's height, frame by frame.
  await expect.poll(() => heldStyle(root)).toEqual(['', '']);
  expect(await frames(root, 'height')).toBeNull();
  expect(await opacity(content)).toBeCloseTo(incoming);
  expect(await opacity(copies)).toBeCloseTo(outgoing);

  await finishAnimations(page);
  await expect(panel(page).layer).toHaveCount(0);
  await expect(list(page).layer).toHaveCount(0);
  expect(await heldStyle(list(page).root)).toEqual(['', '']);
});

test('a season tab replaces a filter that is still gliding', async ({
  page,
}) => {
  await open(page);
  await watched(page, 'First Contact').click();
  await expect(filter(page, /^Unwatched 3/)).toBeVisible();
  await pauseAnimations(page);
  await filter(page, /^Unwatched 3/).click();
  await expect(list(page).copies).toHaveCount(1);
  await expect.poll(() => frames(row(page, 1, 2), 'transform')).not.toBeNull();
  await seekAnimations(page, 0.1);
  const id = await row(page, 1, 2).getAttribute('data-episode');
  const moved = await row(page, 1, 2).evaluate(
    (node) => getComputedStyle(node).transform,
  );
  const faded = await opacity(list(page).copies);
  expect(moved).not.toBe('none');
  expect(faded).toBeGreaterThan(0);
  expect(faded).toBeLessThan(1);

  await tab(page, 'Season 2').click();
  const { root, content, copies } = panel(page);
  await expect(copies).toHaveCount(1);
  // The copy of the old panel shows the list where it was, leaving row included.
  expect(
    await copies
      .locator(`[data-episode="${id}"]`)
      .evaluate((node: HTMLElement) => node.style.transform),
  ).toBe(moved);
  expect(
    await opacity(copies.locator('[data-episode="episode-0"]')),
  ).toBeCloseTo(faded);
  await expect(copies.locator('[data-motion-layer]')).toHaveCount(0);
  await expect(list(page).layer).toHaveCount(0);
  expect(await heldStyle(list(page).root)).toEqual(['', '']);
  // The new height has to be measured without the old list's.
  await expect.poll(() => frames(root, 'height')).not.toBeNull();
  const height = await frames(root, 'height');
  expect(parseFloat(height![1])).toBeCloseTo(
    await content.evaluate((node) => node.getBoundingClientRect().height),
  );
});

test('nothing animates under reduced motion', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await watchLayers(page);
  await open(page);
  await watched(page, 'First Contact').click();
  await expect(filter(page, /^Unwatched 3/)).toBeVisible();
  await filter(page, /^Unwatched 3/).click();
  await expect(listed(page)).toHaveCount(3);
  await tab(page, 'Season 2').click();
  await expect(listed(page)).toHaveCount(6);
  await nextFrame(page);
  expect(await layerSeen(page)).toBeUndefined();
});

test('a link to an episode opens its season without animating', async ({
  page,
}) => {
  await watchLayers(page);
  await page.goto('/#library/shows/show-0/season-1/episode-7');
  await expect(tab(page, 'Season 2')).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('[data-episode="episode-7"]')).toBeInViewport();
  await nextFrame(page);
  expect(await layerSeen(page)).toBeUndefined();
});
