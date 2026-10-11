import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

async function openVideo(page: Page) {
  await page.goto('/#online/youtube');
  await page
    .getByRole('button', {
      name: 'Play: A Quiet Journey · Episode 5',
      exact: true,
    })
    .click();
  const video = page.locator('video');
  await expect
    .poll(() =>
      video.evaluate((element: HTMLVideoElement) => element.readyState),
    )
    .toBeGreaterThanOrEqual(2);
  return video;
}

test('desktop video volume and mute are remembered across consecutive plays and reloads', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const video = await openVideo(page);
  await page.getByRole('slider', { name: 'Playback volume' }).fill('0.23');
  await expect(video).toHaveJSProperty('volume', 0.23);
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .audio_volume,
    )
    .toBe(0.23);
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  await expect(video).toHaveJSProperty('muted', true);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page
    .getByRole('button', {
      name: 'Play: After the Rain · Episode 6',
      exact: true,
    })
    .click();
  await expect(video).toHaveJSProperty('volume', 0.23);
  await expect(video).toHaveJSProperty('muted', true);
  await page.reload();
  await page
    .getByRole('button', {
      name: 'Play: A Quiet Journey · Episode 5',
      exact: true,
    })
    .click();
  await expect(video).toHaveJSProperty('volume', 0.23);
  await expect(video).toHaveJSProperty('muted', true);
  await page.getByRole('button', { name: 'Unmute', exact: true }).click();
  await expect(video).toHaveJSProperty('muted', false);
  await expect(
    page.getByRole('slider', { name: 'Playback volume' }),
  ).toHaveValue('0.23');
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  await page.getByRole('slider', { name: 'Playback volume' }).fill('0.37');
  await expect(video).toHaveJSProperty('volume', 0.37);
  await expect(video).toHaveJSProperty('muted', false);
  await page.getByRole('button', { name: 'Close player', exact: true }).click();
  await page
    .getByRole('button', {
      name: 'Play: After the Rain · Episode 6',
      exact: true,
    })
    .click();
  await expect(video).toHaveJSProperty('volume', 0.37);
  await expect(video).toHaveJSProperty('muted', false);
  await page.screenshot({ path: info.outputPath('remembered-volume.png') });
});

test('desktop window resizing does not automatically enter fullscreen', async ({
  page,
}, info) => {
  await openVideo(page);
  await page.setViewportSize({ width: 900, height: 500 });
  await expect(
    page.getByRole('button', { name: 'Fullscreen', exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => document.fullscreenElement)).toBeNull();
  await expect(
    page.getByRole('region', { name: 'Media player' }),
  ).not.toHaveCSS('height', '500px');
  await page.screenshot({ path: info.outputPath('desktop-landscape.png') });
});

test('desktop watchlists retain hover actions and immediate row dragging', async ({
  page,
}, info) => {
  await page.goto('/#online/youtube');
  await page
    .getByRole('button', { name: 'Open watchlists', exact: true })
    .click();
  const sidebar = page.locator('[data-youtube-watchlist-frame]');
  const rows = sidebar.locator('[data-watchlist-video-id]');
  const ids = () =>
    rows.evaluateAll((elements) =>
      elements.map(
        (element) => (element as HTMLElement).dataset.watchlistVideoId!,
      ),
    );
  const initial = await ids();
  await expect(sidebar.getByRole('button', { name: /^Reorder / })).toHaveCount(
    0,
  );
  await rows.first().hover();
  await expect(
    rows.first().getByRole('button', { name: 'Mark watched', exact: true }),
  ).toBeVisible();
  await expect(
    rows.first().getByRole('button', { name: 'Download', exact: true }),
  ).toBeVisible();
  await expect(
    rows
      .first()
      .getByRole('button', { name: 'Remove from watchlist', exact: true }),
  ).toBeVisible();
  const source = (await rows.first().boundingBox())!,
    target = (await rows.nth(2).boundingBox())!;
  await page.mouse.move(source.x + 30, source.y + source.height / 2);
  await page.mouse.down();
  await page.mouse.move(source.x + 30, source.y + source.height / 2 + 10);
  await page.mouse.move(target.x + 30, target.y + target.height - 5, {
    steps: 6,
  });
  await page.mouse.up();
  await expect
    .poll(ids)
    .toEqual([initial[1], initial[2], initial[0], ...initial.slice(3)]);
  await expect(
    sidebar
      .locator('[data-playing]')
      .filter({ hasText: 'A Quiet Journey · Episode 5' }),
  ).toHaveCSS('opacity', '1');
  await page.screenshot({
    path: info.outputPath('desktop-watchlist-drag.png'),
  });
});

test.describe('mobile player gestures', () => {
  test.use({
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });

  test('mobile playback uses device volume and leaves desktop volume and mute preferences intact', async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    const video = await openVideo(page);
    await page.getByRole('slider', { name: 'Playback volume' }).fill('0.23');
    await page.getByRole('button', { name: 'Mute', exact: true }).click();
    await expect(video).toHaveJSProperty('muted', true);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.locator('.volume-controls')).toHaveCount(0);
    await expect(video).toHaveJSProperty('volume', 1);
    await expect(video).toHaveJSProperty('muted', false);
    await page.getByRole('button', { name: 'Close player', exact: true }).tap();
    await page
      .getByRole('button', {
        name: 'Play: After the Rain · Episode 6',
        exact: true,
      })
      .tap();
    await expect(video).toHaveJSProperty('volume', 1);
    await expect(video).toHaveJSProperty('muted', false);
    await page.reload();
    await page
      .getByRole('button', {
        name: 'Play: A Quiet Journey · Episode 5',
        exact: true,
      })
      .tap();
    await expect(video).toHaveJSProperty('volume', 1);
    await expect(video).toHaveJSProperty('muted', false);
    await expect(page.locator('.volume-controls')).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath('device-volume-mobile.png'),
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await expect(video).toHaveJSProperty('volume', 0.23);
    await expect(video).toHaveJSProperty('muted', true);
    await expect(
      page.getByRole('button', { name: 'Unmute', exact: true }),
    ).toBeVisible();
    expect(
      (await (await page.request.get('/api/v1/me/appearance')).json())
        .audio_volume,
    ).toBe(0.23);
    await page.screenshot({
      path: info.outputPath('device-volume-desktop-preserved.png'),
    });
  });

  test('closing pinned playback moves the page and open watchlist with the player', async ({
    page,
  }, info) => {
    await openVideo(page);
    const player = page.getByRole('region', { name: 'Media player' });
    await expect(player).toHaveCSS('transform', 'none');
    const header = page.locator('.page-header');
    await expect
      .poll(async () => (await header.boundingBox())!.y)
      .toBeCloseTo((390 * 9) / 16, 0);
    await page
      .getByRole('button', { name: 'Open watchlists', exact: true })
      .tap();
    const drawer = page.getByRole('dialog', {
      name: 'YouTube watchlists',
      exact: true,
    });
    await expect(drawer).toBeVisible();
    await page.evaluate(() => {
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (keyframes, options) {
        const animation = animate.call(this, keyframes, options);
        if (
          this.classList.contains('player') &&
          Number(animation.effect?.getTiming().duration) > 0
        )
          animation.pause();
        return animation;
      };
      const main = document.querySelector('main.content')!;
      main.addEventListener('transitionrun', () => {
        for (const animation of main.getAnimations()) animation.pause();
      });
    });
    const openHeight = (await player.boundingBox())!.height;
    await page.getByRole('button', { name: 'Close player', exact: true }).tap();
    await expect
      .poll(() =>
        player.evaluate((element) =>
          element
            .getAnimations()
            .some(
              (animation) => Number(animation.effect?.getTiming().duration) > 0,
            ),
        ),
      )
      .toBe(true);
    await page.evaluate(() => {
      for (const element of document.querySelectorAll(
        '.player, main.content',
      )) {
        for (const animation of element.getAnimations()) {
          animation.pause();
          animation.currentTime =
            Number(animation.effect!.getTiming().duration) / 2;
        }
      }
    });
    const bounds = (await player.boundingBox())!;
    const edge = bounds.y + bounds.height;
    const headerTop = (await header.boundingBox())!.y;
    const drawerTop = (await drawer.boundingBox())!.y;
    expect(edge).toBeGreaterThan(0);
    expect(edge).toBeLessThan(openHeight);
    expect(Math.abs(headerTop - edge)).toBeLessThan(2);
    expect(Math.abs(drawerTop - edge)).toBeLessThan(2);
    await info.attach('closing-geometry', {
      body: JSON.stringify(
        { openHeight, playerBottom: edge, headerTop, drawerTop },
        null,
        2,
      ),
      contentType: 'application/json',
    });
    await page.screenshot({
      path: info.outputPath('mobile-close-midpoint.png'),
    });
    await page.evaluate(() => {
      for (const element of document.querySelectorAll(
        '.player, main.content',
      )) {
        for (const animation of element.getAnimations()) animation.finish();
      }
    });
    await expect(player).toHaveCount(0);
    await expect.poll(async () => (await header.boundingBox())!.y).toBe(0);
    await expect.poll(async () => (await drawer.boundingBox())!.y).toBe(0);
    await page.screenshot({
      path: info.outputPath('mobile-close-finished.png'),
    });
  });

  test('side taps reveal controls, defer pause, and seek ten seconds for every additional tap', async ({
    page,
  }, info) => {
    const video = await openVideo(page);
    const controls = page.locator('.player-controls');
    await page.getByRole('slider', { name: 'Playback position' }).fill('30');
    const side = (right: boolean) =>
      video.tap({ position: { x: right ? 340 : 50, y: 95 } });
    await side(true);
    await expect(video).toHaveJSProperty('paused', false);
    await side(true);
    await expect
      .poll(() =>
        video.evaluate((element: HTMLVideoElement) => element.currentTime),
      )
      .toBeGreaterThan(39);
    await side(true);
    await expect
      .poll(() =>
        video.evaluate((element: HTMLVideoElement) => element.currentTime),
      )
      .toBeGreaterThan(49);
    await page.waitForTimeout(450);
    await expect(video).toHaveJSProperty('paused', false);
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 5000 });
    const beforeReveal = await video.evaluate(
      (element: HTMLVideoElement) => element.currentTime,
    );
    await side(false);
    await expect(controls).toHaveCSS('opacity', '1');
    await page.waitForTimeout(450);
    await expect(video).toHaveJSProperty('paused', false);
    expect(
      await video.evaluate((element: HTMLVideoElement) => element.currentTime),
    ).toBeLessThan(beforeReveal + 3);
    await side(false);
    await page.waitForTimeout(100);
    await expect(video).toHaveJSProperty('paused', false);
    await expect(video).toHaveJSProperty('paused', true);
    const beforeSeek = await video.evaluate(
      (element: HTMLVideoElement) => element.currentTime,
    );
    await side(false);
    await side(false);
    await side(false);
    await expect
      .poll(() =>
        video.evaluate((element: HTMLVideoElement) => element.currentTime),
      )
      .toBeCloseTo(beforeSeek - 20, 0);
    await page.waitForTimeout(450);
    await expect(video).toHaveJSProperty('paused', true);
    await video.tap({ position: { x: 195, y: 95 } });
    await expect(video).toHaveJSProperty('paused', false);
    await page.getByRole('button', { name: 'Pause', exact: true }).tap();
    await expect(video).toHaveJSProperty('paused', true);
    await page.screenshot({
      path: info.outputPath('mobile-seek-gestures.png'),
    });
  });

  test('landscape rotation fills the viewport without a tap and portrait restores the pinned player', async ({
    page,
  }, info) => {
    const fullscreenWarnings: string[] = [];
    page.on('console', (message) => {
      if (
        message.text().includes('requestFullscreen') &&
        message.text().includes('user gesture')
      )
        fullscreenWarnings.push(message.text());
    });
    const cdp = await page.context().newCDPSession(page);
    const rotate = async (landscape: boolean) => {
      const native = await cdp.send('Runtime.evaluate', {
        expression: '!!document.fullscreenElement',
        userGesture: false,
        returnByValue: true,
      });
      const metrics = {
        width: landscape ? 844 : 390,
        height: landscape ? 390 : 844,
        screenWidth: landscape ? 844 : 390,
        screenHeight: landscape ? 390 : 844,
        deviceScaleFactor: 1,
        mobile: true,
        screenOrientation: {
          type: landscape
            ? ('landscapePrimary' as const)
            : ('portraitPrimary' as const),
          angle: landscape ? 90 : 0,
        },
      };
      if (native.result.value) {
        await cdp.send('Emulation.setDeviceMetricsOverride', metrics);
        await expect(
          page.getByRole('button', { name: 'Fullscreen', exact: true }),
        ).toHaveCount(1);
      }
      await page.setViewportSize({
        width: landscape ? 844 : 390,
        height: landscape ? 390 : 844,
      });
      await cdp.send('Emulation.setDeviceMetricsOverride', metrics);
    };
    await rotate(false);
    const video = await openVideo(page);
    await page
      .getByRole('button', { name: 'Open watchlists', exact: true })
      .click();
    const drawer = page.getByRole('dialog', {
      name: 'YouTube watchlists',
      exact: true,
    });
    await expect(drawer).toBeVisible();
    // Let transient user activation expire before rotating, as it does during normal viewing.
    await page.waitForTimeout(6000);
    const activation = await cdp.send('Runtime.evaluate', {
      expression: 'navigator.userActivation.isActive',
      userGesture: false,
      returnByValue: true,
    });
    expect(activation.result.value).toBe(false);
    const player = page.getByRole('region', { name: 'Media player' });
    const controls = page.locator('.player-controls');
    await expect(controls).toHaveCSS('opacity', '0');
    await rotate(true);
    await expect(
      page.getByRole('button', { name: 'Exit fullscreen', exact: true }),
    ).toHaveCount(1);
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 500 });
    await expect
      .poll(async () => (await player.boundingBox())!.width)
      .toBe(844);
    await expect
      .poll(async () => (await player.boundingBox())!.height)
      .toBe(390);
    await expect(video).toHaveJSProperty('paused', false);
    await page.screenshot({
      path: info.outputPath('mobile-rotation-landscape.png'),
    });
    await video.tap({ position: { x: 422, y: 150 } });
    await expect(video).toHaveJSProperty('paused', false);
    await page
      .getByRole('button', { name: 'Exit fullscreen', exact: true })
      .tap();
    await expect(
      page.getByRole('button', { name: 'Fullscreen', exact: true }),
    ).toBeVisible();
    // Resizing or switching videos in the same orientation must not reopen fullscreen after an explicit exit.
    await page.waitForTimeout(450);
    await expect(
      page.getByRole('button', { name: 'Fullscreen', exact: true }),
    ).toBeVisible();
    await rotate(false);
    // Native fullscreen is also allowed when rotation follows a recent tap.
    await video.tap({ position: { x: 195, y: 100 } });
    await expect(video).toHaveJSProperty('paused', true);
    await page.getByRole('button', { name: 'Play', exact: true }).tap();
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 5000 });
    await rotate(true);
    await expect(
      page.getByRole('button', { name: 'Exit fullscreen', exact: true }),
    ).toHaveCount(1);
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 500 });
    expect(await page.evaluate(() => document.fullscreenElement !== null)).toBe(
      true,
    );
    await rotate(false);
    await expect(
      page.getByRole('button', { name: 'Fullscreen', exact: true }),
    ).toHaveCount(1);
    await expect(controls).toHaveCSS('opacity', '0', { timeout: 500 });
    await expect
      .poll(async () => (await player.boundingBox())!.height)
      .toBeCloseTo((390 * 9) / 16, 0);
    await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
    await expect(drawer).toBeVisible();
    await expect(drawer).toHaveCSS('width', '390px');
    await expect
      .poll(async () => (await drawer.boundingBox())!.y)
      .toBeCloseTo((390 * 9) / 16, 0);
    await page.screenshot({
      path: info.outputPath('mobile-rotation-portrait.png'),
    });
    expect(fullscreenWarnings).toEqual([]);
    await cdp.detach();
  });

  test('held watchlist handles reorder while ordinary swipes keep scrolling', async ({
    page,
  }, info) => {
    await page.setViewportSize({ width: 390, height: 480 });
    await page.goto('/#online/youtube');
    await page
      .getByRole('button', { name: 'Open watchlists', exact: true })
      .click();
    const drawer = page.getByRole('dialog', {
      name: 'YouTube watchlists',
      exact: true,
    });
    const rows = drawer.locator('[data-watchlist-video-id]');
    const ids = () =>
      rows.evaluateAll((elements) =>
        elements.map(
          (element) => (element as HTMLElement).dataset.watchlistVideoId!,
        ),
      );
    const initial = await ids();
    const first = rows.first();
    const handle = first.getByRole('button', { name: /^Reorder / });
    await expect(handle).toBeVisible();
    const cdp = await page.context().newCDPSession(page);
    const start = async (x: number, y: number) =>
      cdp.send('Input.dispatchTouchEvent', {
        type: 'touchStart',
        touchPoints: [{ x, y }],
      });
    const move = async (x: number, y: number) =>
      cdp.send('Input.dispatchTouchEvent', {
        type: 'touchMove',
        touchPoints: [{ x, y }],
      });
    const end = async () =>
      cdp.send('Input.dispatchTouchEvent', {
        type: 'touchEnd',
        touchPoints: [],
      });
    const bounds = (await handle.boundingBox())!;
    const point = {
      x: bounds.x + bounds.width / 2,
      y: bounds.y + bounds.height / 2,
    };
    await start(point.x, point.y);
    await expect(first.locator('.opacity-35')).toBeVisible();
    const target = (await rows.nth(2).boundingBox())!;
    await move(point.x, target.y + target.height - 5);
    await end();
    await expect
      .poll(ids)
      .toEqual([initial[1], initial[2], initial[0], ...initial.slice(3)]);
    const list = (
      await (await page.request.get('/api/v1/online/youtube/watchlists')).json()
    )[0];
    expect(list.items.map((item: { videoId: string }) => item.videoId)).toEqual(
      await ids(),
    );
    await expect(
      drawer.getByRole('button', {
        name: 'Reorder A Quiet Journey · Episode 5',
        exact: true,
      }),
    ).toHaveAttribute('aria-pressed', 'false');
    await expect(
      drawer
        .locator('[data-playing]')
        .filter({ hasText: 'A Quiet Journey · Episode 5' }),
    ).toHaveCSS('opacity', '1');
    await expect(
      page.getByRole('region', { name: 'Media player' }),
    ).toHaveCount(0);
    await expect(
      drawer.getByRole('dialog', { name: /^Actions for/ }),
    ).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath('mobile-watchlist-reordered.png'),
    });
    const scroll = drawer.getByRole('list');
    const beforeSwipe = await ids();
    const topHandle = rows.nth(2).getByRole('button', { name: /^Reorder / });
    const topBounds = (await topHandle.boundingBox())!;
    const sx = topBounds.x + topBounds.width / 2,
      sy = topBounds.y + topBounds.height / 2;
    await start(sx, sy);
    await move(sx, sy - 35);
    await move(sx, sy - 100);
    await end();
    await expect
      .poll(() => scroll.evaluate((element) => element.scrollTop))
      .toBeGreaterThan(0);
    expect(await ids()).toEqual(beforeSwipe);
    await page.screenshot({
      path: info.outputPath('mobile-watchlist-handle-scroll.png'),
    });
    await drawer
      .getByRole('button', { name: 'Sort watchlist', exact: true })
      .click();
    await drawer
      .getByRole('button', { name: 'Date descending', exact: true })
      .click();
    await expect(rows.getByRole('button', { name: /^Reorder / })).toHaveCount(
      0,
    );
    await cdp.detach();
  });
});
