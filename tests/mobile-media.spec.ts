import { expect, test } from '@playwright/test';
import { hold } from './helpers/touch';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
  await request.patch('/api/v1/me/appearance', {
    data: { poster_columns: 5, video_columns: 7 },
  });
});

test('mobile media grids use three posters or two videos and retain desktop preferences', async ({
  page,
}, info) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const evidence: { width: number; route: string; columns: number }[] = [];
  for (const width of [390, 1440]) {
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
  for (const width of [390, 720]) {
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
    const settings = drawer.getByRole('dialog', {
      name: 'Watchlist settings',
      exact: true,
    });
    await expect(settings).toBeVisible();
    // The menu's backdrop covers its button and takes the press that closes it.
    await drawer
      .getByRole('button', { name: 'Watchlist settings', exact: true })
      .click({ force: true });
    await expect(settings).toBeHidden();
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
  for (const width of [390, 720]) {
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
    await hold(page, card, () =>
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
    await hold(page, row, () => expect(actions).toBeVisible());
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
    await hold(page, row, () =>
      expect(
        actions.getByRole('button', { name: 'Mark unwatched', exact: true }),
      ).toBeVisible(),
    );
    await actions
      .getByRole('button', { name: 'Download', exact: true })
      .click();
    await hold(page, row, () =>
      expect(
        actions.getByRole('button', { name: 'Delete download', exact: true }),
      ).toBeVisible(),
    );
    await actions
      .getByRole('button', { name: 'Delete download', exact: true })
      .click();
    await hold(page, row, () =>
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

  test('holding a Home card opens its menu, or its details without one, instead of playing it', async ({
    page,
  }) => {
    await page.goto('/#home');
    const player = page.getByRole('region', { name: 'Media player' });
    const watched = page.getByRole('menuitem', { name: 'Mark watched' });
    await hold(
      page,
      page
        .locator('article[data-layout-key="home-continue:youtube:preview0000"]')
        .locator('.card-primary'),
      () => expect(watched).toBeVisible(),
    );
    await expect(player).toHaveCount(0);
    await watched.tap();
    await expect(page.getByRole('menu')).toHaveCount(0);

    await hold(
      page,
      page
        .locator('article[data-layout-key="home-continue:episode-0"]')
        .locator('.card-primary'),
      () => expect(page).toHaveURL(/#library\/shows\/episode-0$/),
    );
    await expect(player).toHaveCount(0);
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
