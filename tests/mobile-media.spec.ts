import { expect, test, type Locator, type Page } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
  await request.patch('/api/v1/me/appearance', {
    data: { poster_columns: 5, video_columns: 7 },
  });
});

test('mobile drawers slide from their own edge on opening, closing and reversal', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#online/youtube');
  await expect(
    page.getByRole('button', { name: 'Open watchlists', exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (frames, options) {
      const animation = animate.call(this, frames, options);
      if (
        this instanceof HTMLDialogElement &&
        Number(animation.effect?.getTiming().duration) > 0
      ) {
        animation.pause();
        animation.currentTime = 0;
      }
      return animation;
    };
  });
  const evidence = [];
  for (const { name, openName, closeName, direction } of [
    {
      name: 'Navigation menu',
      openName: 'Open navigation',
      closeName: 'Close navigation',
      direction: -1,
    },
    {
      name: 'YouTube watchlists',
      openName: 'Open watchlists',
      closeName: 'Close watchlists',
      direction: 1,
    },
  ]) {
    const drawer = page.getByRole('dialog', { name, exact: true });
    const open = page.getByRole('button', { name: openName, exact: true });
    const mainBounds = await page.locator('main').boundingBox();
    const moving = () =>
      drawer.evaluate((node) =>
        node
          .getAnimations()
          .some(
            (animation) =>
              Number(animation.effect?.getTiming().duration) > 0 &&
              animation.effect instanceof KeyframeEffect &&
              !animation.effect.pseudoElement,
          ),
      );
    const sample = async (fraction: number) =>
      drawer.evaluate((node, fraction) => {
        for (const motion of document
          .getAnimations()
          .filter(
            (animation) =>
              animation.effect instanceof KeyframeEffect &&
              animation.effect.target === node,
          ))
          motion.currentTime =
            Number(motion.effect!.getTiming().duration) * fraction;
        const { x, y, width, height } = node.getBoundingClientRect();
        return {
          x,
          y,
          width,
          height,
          backdrop: Number(getComputedStyle(node, '::backdrop').opacity),
        };
      }, fraction);
    const finish = () =>
      drawer.evaluate((node) =>
        document
          .getAnimations()
          .filter(
            (animation) =>
              animation.effect instanceof KeyframeEffect &&
              animation.effect.target === node,
          )
          .forEach((animation) => animation.finish()),
      );
    await open.click();
    await expect.poll(moving).toBe(true);
    const opening = [];
    for (const fraction of [0, 0.2, 0.4, 0.6, 0.8, 0.98])
      opening.push(await sample(fraction));
    expect(opening[0].x * direction).toBeCloseTo(opening[0].width, 0);
    expect(Math.abs(opening.at(-1)!.x)).toBeLessThan(1);
    if (direction === -1) {
      expect(opening[0].backdrop).toBe(0);
      expect(opening.at(-1)!.backdrop).toBeCloseTo(1, 3);
    }
    for (let i = 1; i < opening.length; i++) {
      expect(opening[i].x * direction).toBeLessThan(
        opening[i - 1].x * direction,
      );
      expect(opening[i].width).toBeCloseTo(opening[0].width, 3);
      expect(opening[i].y).toBeCloseTo(opening[0].y, 3);
      expect(opening[i].height).toBeCloseTo(opening[0].height, 3);
      if (direction === -1)
        expect(opening[i].backdrop).toBeGreaterThan(opening[i - 1].backdrop);
    }
    await sample(0.2);
    await page.screenshot({ path: info.outputPath(`${name}-opening.png`) });
    await finish();
    await expect.poll(moving).toBe(false);
    await drawer.getByRole('button', { name: closeName, exact: true }).click();
    await expect.poll(moving).toBe(true);
    const closing = [];
    for (const fraction of [0, 0.2, 0.4, 0.6, 0.8, 0.98])
      closing.push(await sample(fraction));
    expect(closing[0].x).toBeCloseTo(0, 0);
    expect(closing.at(-1)!.x * direction).toBeCloseTo(closing[0].width, 0);
    if (direction === -1) {
      expect(closing[0].backdrop).toBe(1);
      expect(closing.at(-1)!.backdrop).toBeCloseTo(0, 3);
    }
    for (let i = 1; i < closing.length; i++) {
      expect(closing[i].x * direction).toBeGreaterThan(
        closing[i - 1].x * direction,
      );
      expect(closing[i].width).toBeCloseTo(closing[0].width, 3);
      if (direction === -1)
        expect(closing[i].backdrop).toBeLessThan(closing[i - 1].backdrop);
    }
    await sample(0.2);
    await page.screenshot({ path: info.outputPath(`${name}-closing.png`) });
    const beforeReversal = await sample(0.4);
    // The modal keeps the page inert during its exit; request reopening without
    // waiting for that exit to finish, as a rapid toggle does.
    await open.evaluate((button: HTMLButtonElement) => button.click());
    await expect(drawer).not.toHaveAttribute('inert', '');
    await expect.poll(moving).toBe(true);
    const afterReversal = await sample(0);
    expect(Math.abs(afterReversal.x - beforeReversal.x)).toBeLessThan(1);
    if (direction === -1)
      expect(afterReversal.backdrop).toBeCloseTo(beforeReversal.backdrop, 3);
    await finish();
    await expect.poll(moving).toBe(false);
    expect(await page.locator('main').boundingBox()).toEqual(mainBounds);
    await drawer.getByRole('button', { name: closeName, exact: true }).click();
    await expect.poll(moving).toBe(true);
    await finish();
    await expect(drawer).toBeHidden();
    await expect(open).toBeFocused();
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await open.click();
    await expect(drawer).toBeVisible();
    expect((await drawer.boundingBox())!.x).toBeCloseTo(0, 0);
    expect(await moving()).toBe(false);
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(open).toBeFocused();
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    evidence.push({ name, opening, closing, beforeReversal, afterReversal });
  }
  await info.attach('mobile-drawer-motion', {
    body: JSON.stringify(evidence, null, 2),
    contentType: 'application/json',
  });
});

test('drawer slides start by the first frame after each click', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#online/youtube');
  await expect(
    page.getByRole('button', { name: 'Open watchlists', exact: true }),
  ).toBeVisible();
  const evidence = [];
  for (const { name, openName, closeName, direction } of [
    {
      name: 'Navigation menu',
      openName: 'Open navigation',
      closeName: 'Close navigation',
      direction: -1,
    },
    {
      name: 'YouTube watchlists',
      openName: 'Open watchlists',
      closeName: 'Close watchlists',
      direction: 1,
    },
  ]) {
    const drawer = page.getByRole('dialog', { name, exact: true });
    for (const opening of [true, false, true, false]) {
      const button = opening
        ? page.getByRole('button', { name: openName, exact: true })
        : drawer.getByRole('button', { name: closeName, exact: true });
      await button.evaluate(
        (button, { name, opening, direction }) => {
          button.addEventListener(
            'click',
            () => {
              const clickedAt = performance.now();
              requestAnimationFrame(() => {
                const dialog = [...document.querySelectorAll('dialog')].find(
                  (node) => node.getAttribute('aria-label') === name,
                )!;
                const bounds = dialog.getBoundingClientRect();
                const started = dialog
                  .getAnimations()
                  .some(
                    (animation) =>
                      animation.effect instanceof KeyframeEffect &&
                      !animation.effect.pseudoElement &&
                      Number(animation.effect.getTiming().duration) > 0,
                  );
                document.documentElement.dataset.drawerFirstFrame =
                  JSON.stringify({
                    name,
                    opening,
                    delayMs: performance.now() - clickedAt,
                    started,
                    distance: bounds.x * direction,
                    width: bounds.width,
                  });
              });
            },
            { once: true },
          );
          delete document.documentElement.dataset.drawerFirstFrame;
        },
        { name, opening, direction },
      );
      await button.click();
      await expect
        .poll(() =>
          page.evaluate(
            () => document.documentElement.dataset.drawerFirstFrame,
          ),
        )
        .toBeTruthy();
      const frame = await page.evaluate(() =>
        JSON.parse(document.documentElement.dataset.drawerFirstFrame!),
      );
      expect(frame.started).toBe(true);
      if (opening) expect(frame.distance).toBeLessThan(frame.width);
      else expect(frame.distance).toBeGreaterThan(0);
      evidence.push(frame);
      if (opening) await expect(drawer).toHaveCSS('transform', 'none');
      else await expect(drawer).toBeHidden();
    }
  }
  await info.attach('drawer-first-frames', {
    body: JSON.stringify(evidence, null, 2),
    contentType: 'application/json',
  });
});

test('mobile media grids use three posters or two videos and retain desktop preferences', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const evidence: { width: number; route: string; columns: number }[] = [];
  for (const width of [320, 390, 720, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const route of [
      'library/movies',
      'library/shows',
      'library/music',
      'online/youtube',
      'online/twitch',
      'online/kick',
    ]) {
      const posters = route.startsWith('library/');
      const expected = width <= 720 ? (posters ? 3 : 2) : posters ? 5 : 7;
      await page.goto(`/#${route}`);
      const grid = page.locator('[data-card-grid]').first();
      await expect(grid).toBeVisible();
      const columns = () =>
        grid.evaluate(
          (element) =>
            getComputedStyle(element).gridTemplateColumns.split(' ').length,
        );
      await expect.poll(columns).toBe(expected);
      await expect
        .poll(() =>
          page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        )
        .toBe(true);
      if (width <= 720) {
        await grid.dispatchEvent('wheel', { ctrlKey: true, deltaY: -100 });
        await expect.poll(columns).toBe(expected);
      }
      evidence.push({ width, route, columns: await columns() });
      await page.screenshot({
        path: info.outputPath(`${route.replaceAll('/', '-')}-${width}.png`),
      });
    }
  }
  expect(
    await (await page.request.get('/api/v1/me/appearance')).json(),
  ).toMatchObject({
    poster_columns: 5,
    video_columns: 7,
  });
  await info.attach('responsive-columns', {
    body: JSON.stringify(evidence, null, 2),
    contentType: 'application/json',
  });
  expect(errors).toEqual([]);
  expect(
    (await (await page.request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
});

test('mobile watchlists stay below pinned playback and preserve the desktop sidebar', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto('/#online/youtube');
  await page
    .getByRole('button', { name: 'Open watchlists', exact: true })
    .click();
  const desktopFrame = page.locator('[data-youtube-watchlist-frame]');
  await expect(desktopFrame).toBeVisible();
  const desktopWidth = (await desktopFrame.boundingBox())!.width;
  const drawer = page.getByRole('dialog', {
    name: 'YouTube watchlists',
    exact: true,
  });
  const open = page.getByRole('button', {
    name: 'Open watchlists',
    exact: true,
  });
  for (const width of [320, 390, 720]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(drawer).toBeHidden();
    await expect(open).toBeVisible();
    const grid = page.locator('[data-card-grid]').first();
    await expect(grid).toBeVisible();
    await expect
      .poll(
        async () =>
          (await grid.boundingBox())!.width -
          (await page
            .locator('[data-feed-content]')
            .evaluate((element) => element.clientWidth)),
      )
      .toBeCloseTo(0, 0);
    const feedBounds = await grid.boundingBox();
    await open.click();
    await expect(drawer).toBeVisible();
    await expect(drawer).toHaveCSS('width', `${width}px`);
    await expect(drawer).toHaveCSS('height', '900px');
    expect(await drawer.evaluate((element) => element.matches(':modal'))).toBe(
      false,
    );
    expect(await grid.boundingBox()).toEqual(feedBounds);
    await drawer
      .getByRole('button', { name: 'Watchlist settings', exact: true })
      .click();
    await expect(
      drawer.getByRole('dialog', { name: 'Watchlist settings', exact: true }),
    ).toBeVisible();
    await drawer
      .getByRole('button', { name: 'Watchlist settings', exact: true })
      .click();
    await page.screenshot({
      path: info.outputPath(`mobile-watchlist-${width}.png`),
    });
    await drawer
      .getByRole('button', { name: 'Close watchlists', exact: true })
      .click();
    await expect(drawer).toBeHidden();
    await expect(open).toBeFocused();
    await open.click();
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(open).toBeFocused();
  }
  await page.setViewportSize({ width: 390, height: 700 });
  await open.click();
  // Narrow windows with a mouse keep hover actions instead of touch sheets and grips.
  const firstRow = drawer.locator('[data-watchlist-video-id]').first();
  await expect(drawer.getByRole('button', { name: /^Reorder / })).toHaveCount(
    0,
  );
  await firstRow.hover();
  await expect(
    firstRow.getByRole('button', { name: 'Mark watched', exact: true }),
  ).toBeVisible();
  const video = drawer
    .locator('[data-watchlist-video-id] [role="button"]')
    .first();
  await expect(video).toBeVisible();
  await video.click();
  await expect(drawer).toBeVisible();
  await expect(
    page.getByRole('region', { name: 'Media player' }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page
        .locator('video')
        .evaluate((video: HTMLVideoElement) => video.currentTime),
    )
    .toBeGreaterThan(0);
  const player = page.getByRole('region', { name: 'Media player' });
  await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
  await expect
    .poll(async () => (await drawer.boundingBox())!.y)
    .toBeCloseTo((390 * 9) / 16, 0);
  const watchlistScroll = drawer.getByRole('list');
  await watchlistScroll.evaluate((element) => {
    element.scrollTop = 200;
  });
  expect(
    await watchlistScroll.evaluate((element) => element.scrollTop),
  ).toBeGreaterThan(0);
  await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
  await page.screenshot({
    path: info.outputPath('mobile-watchlist-playback.png'),
  });
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await expect(drawer).toBeVisible();
  await expect.poll(async () => (await drawer.boundingBox())!.y).toBe(0);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(drawer).toBeHidden();
  await expect(desktopFrame).toBeVisible();
  await expect
    .poll(async () => (await desktopFrame.boundingBox())!.width)
    .toBe(desktopWidth);
  await desktopFrame
    .locator('[data-watchlist-video-id] [role="button"]')
    .first()
    .click();
  await expect(
    page.getByRole('region', { name: 'Media player' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Close watchlists', exact: true }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page.screenshot({ path: info.outputPath('desktop-watchlist.png') });
  expect(errors).toEqual([]);
  expect(
    (await (await page.request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
});

test('mobile player fits the video width, stays above feed scrolling and ignores saved height', async ({
  page,
}, info) => {
  await page.request.patch('/api/v1/me/appearance', {
    data: { player_height: 480 },
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#online/youtube');
  await page
    .getByRole('button', {
      name: 'Play: A Quiet Journey · Episode 5',
      exact: true,
    })
    .click();
  const player = page.getByRole('region', { name: 'Media player' });
  const video = player.locator('video');
  await expect
    .poll(() =>
      video.evaluate((element: HTMLVideoElement) => element.readyState),
    )
    .toBeGreaterThanOrEqual(2);
  for (const width of [320, 390, 720]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(
      page.getByRole('slider', { name: 'Resize player height' }),
    ).toHaveCount(0);
    await expect
      .poll(async () => (await player.boundingBox())!.width)
      .toBe(width);
    await expect
      .poll(async () => (await player.boundingBox())!.height)
      .toBeCloseTo((width * 9) / 16, 0);
    await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
    await expect
      .poll(async () => (await page.locator('.page-header').boundingBox())!.y)
      .toBeCloseTo((width * 9) / 16, 0);
    const toolbar = page.getByRole('button', {
      name: 'Sort feed',
      exact: true,
    });
    const toolbarTop = (await toolbar.boundingBox())!.y;
    const feed = page.locator('[data-feed-content]');
    await feed.evaluate((element) => {
      element.scrollTop = 400;
    });
    expect(await feed.evaluate((element) => element.scrollTop)).toBeGreaterThan(
      0,
    );
    await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
    expect((await toolbar.boundingBox())!.y).toBeCloseTo(toolbarTop, 0);
    await page.screenshot({
      path: info.outputPath(`pinned-player-${width}.png`),
    });
  }
  expect(
    await (await page.request.get('/api/v1/me/appearance')).json(),
  ).toMatchObject({ player_height: 480 });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(
    page.getByRole('slider', { name: 'Resize player height' }),
  ).toBeVisible();
  await expect.poll(async () => (await player.boundingBox())!.height).toBe(480);
  await page.screenshot({
    path: info.outputPath('desktop-saved-player-height.png'),
  });
});

async function holdVideo(
  page: Page,
  target: Locator,
  opened: () => Promise<void>,
) {
  await target.scrollIntoViewIfNeeded();
  const bounds = (await target.boundingBox())!;
  const touch = await page.context().newCDPSession(page);
  try {
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [
        { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
      ],
    });
    await opened();
  } finally {
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    });
    await touch.detach();
  }
}

test.describe('mobile video touch actions', () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 844 } });

  test('holding a grid video opens its menu without shortcuts or playback and swipes remain scrolling', async ({
    page,
  }, info) => {
    await page.request.patch('/api/v1/me/appearance', {
      data: { youtube_card_shortcuts: ['download', 'watch_toggle'] },
    });
    await page.goto('/#online/youtube');
    const card = page.locator('article[data-video-id="preview0004"]');
    const menu = page.locator('#video-actions-preview0004');
    await holdVideo(page, card, () =>
      expect(
        menu.getByRole('button', { name: 'Mark watched', exact: true }),
      ).toBeVisible(),
    );
    await expect(
      page.getByRole('region', { name: 'Media player' }),
    ).toHaveCount(0);
    await expect(
      menu.getByRole('button', { name: 'Choose shortcuts…' }),
    ).toHaveCount(0);
    await expect(
      menu.getByRole('button', { name: 'Download', exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('mobile-grid-long-press.png'),
    });
    await menu
      .getByRole('button', { name: 'Mark watched', exact: true })
      .click();
    await expect(menu).toHaveCount(0);
    await expect(card).toHaveCSS('opacity', '0.55');
    const nextCard = page.locator('article[data-video-id="preview0005"]');
    await nextCard.scrollIntoViewIfNeeded();
    const bounds = (await nextCard.boundingBox())!;
    const touch = await page.context().newCDPSession(page);
    const point = {
      x: bounds.x + bounds.width / 2,
      y: bounds.y + bounds.height / 2,
    };
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [point],
    });
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ ...point, y: point.y - 100 }],
    });
    await page.waitForTimeout(700);
    await touch.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
    });
    await touch.detach();
    await expect(page.locator('[id^="video-actions-"]')).toHaveCount(0);
    await expect(
      page.getByRole('region', { name: 'Media player' }),
    ).toHaveCount(0);
  });

  test('holding a watchlist video exposes watched, download and removal actions without playing it', async ({
    page,
  }, info) => {
    await page.goto('/#online/youtube');
    await page
      .getByRole('button', { name: 'Open watchlists', exact: true })
      .click();
    const drawer = page.getByRole('dialog', {
      name: 'YouTube watchlists',
      exact: true,
    });
    const row = drawer.locator(
      '[data-watchlist-video-id="preview0004"] [role="button"]',
    );
    const actions = drawer.getByRole('dialog', {
      name: 'Actions for A Quiet Journey · Episode 5',
      exact: true,
    });
    await holdVideo(page, row, () => expect(actions).toBeVisible());
    await expect(
      page.getByRole('region', { name: 'Media player' }),
    ).toHaveCount(0);
    await expect(
      actions.getByRole('button', { name: 'Mark watched', exact: true }),
    ).toBeVisible();
    await expect(
      actions.getByRole('button', { name: 'Download', exact: true }),
    ).toBeVisible();
    await expect(
      actions.getByRole('button', {
        name: 'Remove from watchlist',
        exact: true,
      }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('mobile-watchlist-long-press.png'),
    });
    await actions
      .getByRole('button', { name: 'Mark watched', exact: true })
      .click();
    await holdVideo(page, row, () =>
      expect(
        actions.getByRole('button', { name: 'Mark unwatched', exact: true }),
      ).toBeVisible(),
    );
    await actions
      .getByRole('button', { name: 'Download', exact: true })
      .click();
    await holdVideo(page, row, () =>
      expect(
        actions.getByRole('button', { name: 'Delete download', exact: true }),
      ).toBeVisible(),
    );
    await actions
      .getByRole('button', { name: 'Delete download', exact: true })
      .click();
    await holdVideo(page, row, () =>
      expect(
        actions.getByRole('button', { name: 'Download', exact: true }),
      ).toBeVisible(),
    );
    await actions
      .getByRole('button', { name: 'Remove from watchlist', exact: true })
      .click();
    await expect(row).toHaveCount(0);
    await expect(drawer).toBeVisible();
    await page.screenshot({
      path: info.outputPath('mobile-watchlist-after-actions.png'),
    });
  });
});

test('desktop video cards retain shortcuts and the shortcut chooser', async ({
  page,
}, info) => {
  await page.request.patch('/api/v1/me/appearance', {
    data: { youtube_card_shortcuts: ['download', 'watch_toggle'] },
  });
  await page.goto('/#online/youtube');
  const card = page.locator('article[data-video-id="preview0004"]');
  await card.hover();
  await expect(
    card.getByRole('button', {
      name: 'Download: A Quiet Journey · Episode 5',
      exact: true,
    }),
  ).toBeVisible();
  await card.getByRole('button', { name: /^More actions for/ }).click();
  await expect(
    page.getByRole('button', { name: /^Choose shortcuts…/ }),
  ).toBeVisible();
  await page.screenshot({ path: info.outputPath('desktop-video-menu.png') });
});
