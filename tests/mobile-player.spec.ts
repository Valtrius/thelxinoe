import { expect, test, type Page } from '@playwright/test';
import { freezeClock } from './helpers/clock';

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

test('touch devices play at device volume and leave the desktop volume and mute intact', async ({
  browser,
}, info) => {
  // Device volume follows touch input rather than width, so each device gets
  // its own context; storage state carries the remembered mute between them.
  const desktop = await browser.newContext({
    hasTouch: false,
    isMobile: false,
    viewport: { width: 1440, height: 1000 },
  });
  let page = await desktop.newPage();
  let video = await openVideo(page);
  await page.getByRole('slider', { name: 'Playback volume' }).fill('0.23');
  await page.getByRole('button', { name: 'Mute', exact: true }).click();
  await expect(video).toHaveJSProperty('muted', true);
  const desktopState = await desktop.storageState();
  await desktop.close();

  const touch = await browser.newContext({
    storageState: desktopState,
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });
  page = await touch.newPage();
  video = await openVideo(page);
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
  await page.screenshot({
    path: info.outputPath('device-volume-mobile.png'),
  });
  const touchState = await touch.storageState();
  await touch.close();

  const restored = await browser.newContext({
    storageState: touchState,
    hasTouch: false,
    isMobile: false,
    viewport: { width: 1440, height: 1000 },
  });
  page = await restored.newPage();
  video = await openVideo(page);
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
  await restored.close();
});

test('pinned playback on library pages fills the screen width from the top edge', async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#library/movies/movie-0');
  await page
    .getByRole('button', { name: 'Play Theatrical', exact: true })
    .click();
  const player = page.getByRole('region', { name: 'Media player' });
  await expect(player).toHaveCSS('transform', 'none');
  await expect.poll(async () => (await player.boundingBox())!.x).toBe(0);
  await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
  await expect.poll(async () => (await player.boundingBox())!.width).toBe(390);
  await expect
    .poll(async () => (await player.boundingBox())!.height)
    .toBeCloseTo((390 * 9) / 16, 0);
  await expect
    .poll(async () => (await page.locator('.page-header').boundingBox())!.y)
    .toBeCloseTo((390 * 9) / 16, 0);
  await page.screenshot({ path: info.outputPath('library-pinned.png') });
});

for (const route of ['home', 'online/youtube'])
  test(`the ${route} player reveals from the top edge and the page follows it without jumping`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`/#${route}`);
    const play = page
      .getByRole('button', { name: /^Play[: ]/ })
      .filter({ visible: true })
      .first();
    // The first opening also loads the player's code.
    await play.click();
    const player = page.getByRole('region', { name: 'Media player' });
    await expect(player).toHaveCSS('transform', 'none');
    const edge = (await page.locator('.player-frame').boundingBox())!.y;
    await page.evaluate(() => {
      const samples: { open: boolean; top: number; edge?: number }[] = [];
      Object.assign(window, { pageEdge: samples });
      const sample = () => {
        const content = document.querySelector('[data-page-content]');
        const frame = document.querySelector('.player-frame');
        if (content)
          samples.push({
            open: !!frame,
            top: content.getBoundingClientRect().top,
            edge: frame?.getBoundingClientRect().top,
          });
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });
    await page
      .getByRole('button', { name: 'Close player', exact: true })
      .click();
    await expect(player).toHaveCount(0);
    await page.waitForTimeout(200);
    await play.click();
    await expect(player).toHaveCSS('transform', 'none');
    const samples = await page.evaluate(
      () =>
        (
          window as unknown as {
            pageEdge: { open: boolean; top: number; edge?: number }[];
          }
        ).pageEdge,
    );
    // The player slides out from and back into the top edge it rests against.
    for (const sample of samples.filter((sample) => sample.open))
      expect(Math.abs(sample.edge! - edge)).toBeLessThan(1);
    const changes = samples.flatMap((sample, index) =>
      index && sample.open !== samples[index - 1].open
        ? [sample.top - samples[index - 1].top]
        : [],
    );
    // Removing the closed player and adding the next one leave the page in place.
    expect(changes).toHaveLength(2);
    for (const change of changes) expect(Math.abs(change)).toBeLessThan(1);
    const tops = samples.map((sample) => sample.top);
    expect(Math.max(...tops) - Math.min(...tops)).toBeGreaterThan(300);
  });

test.describe('mobile player gestures', () => {
  test.use({
    hasTouch: true,
    isMobile: true,
    viewport: { width: 390, height: 844 },
  });

  test('pinned playback moves the page and open watchlist with the player while opening and closing', async ({
    page,
  }, info) => {
    await page.goto('/#online/youtube');
    await page
      .getByRole('button', { name: 'Open watchlists', exact: true })
      .tap();
    const drawer = page.getByRole('dialog', {
      name: 'YouTube watchlists',
      exact: true,
    });
    await expect(drawer).toBeVisible();
    const player = page.getByRole('region', { name: 'Media player' });
    const header = page.locator('.page-header');
    // Hold the player's reveal transitions so their midpoints can be measured.
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
    });
    const revealMidpoint = async () => {
      await expect
        .poll(() =>
          player.evaluate((element) =>
            element
              .getAnimations()
              .some(
                (animation) =>
                  Number(animation.effect?.getTiming().duration) > 0,
              ),
          ),
        )
        .toBe(true);
      await player.evaluate(async (element) => {
        for (const animation of element.getAnimations())
          animation.currentTime =
            Number(animation.effect!.getTiming().duration) / 2;
        // Let the frame's resize observer carry its height into the page.
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        );
      });
    };
    const finishReveal = () =>
      player.evaluate((element) => {
        for (const animation of element.getAnimations()) animation.finish();
      });
    const geometry = async () => {
      const bounds = (await player.boundingBox())!;
      return {
        playerBottom: bounds.y + bounds.height,
        headerTop: (await header.boundingBox())!.y,
        drawerTop: (await drawer.boundingBox())!.y,
      };
    };
    const openHeight = (390 * 9) / 16;
    const expectFollowing = ({
      playerBottom,
      headerTop,
      drawerTop,
    }: Awaited<ReturnType<typeof geometry>>) => {
      expect(playerBottom).toBeGreaterThan(0);
      expect(playerBottom).toBeLessThan(openHeight - 1);
      expect(Math.abs(headerTop - playerBottom)).toBeLessThan(2);
      expect(Math.abs(drawerTop - playerBottom)).toBeLessThan(2);
    };

    await drawer
      .locator('[data-watchlist-video-id] [role="button"]')
      .first()
      .tap();
    await revealMidpoint();
    const opening = await geometry();
    expectFollowing(opening);
    await page.screenshot({
      path: info.outputPath('mobile-open-midpoint.png'),
    });
    await finishReveal();
    await expect(player).toHaveCSS('transform', 'none');
    await expect
      .poll(async () => (await header.boundingBox())!.y)
      .toBeCloseTo(openHeight, 0);
    await expect
      .poll(async () => (await drawer.boundingBox())!.y)
      .toBeCloseTo(openHeight, 0);

    await page.getByRole('button', { name: 'Close player', exact: true }).tap();
    await revealMidpoint();
    const closing = await geometry();
    expectFollowing(closing);
    await info.attach('reveal-geometry', {
      body: JSON.stringify({ openHeight, opening, closing }, null, 2),
      contentType: 'application/json',
    });
    await page.screenshot({
      path: info.outputPath('mobile-close-midpoint.png'),
    });
    await finishReveal();
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
    await page.clock.install();
    const video = await openVideo(page);
    const controls = page.locator('.player-controls');
    const currentTime = () =>
      video.evaluate((element: HTMLVideoElement) => element.currentTime);
    const side = (right: boolean) =>
      video.tap({ position: { x: right ? 340 : 50, y: 95 } });
    await page.getByRole('slider', { name: 'Playback position' }).fill('30');
    await freezeClock(page);
    await side(true);
    await expect(video).toHaveJSProperty('paused', false);
    await side(true);
    await expect.poll(currentTime).toBeGreaterThan(39);
    await side(true);
    await expect.poll(currentTime).toBeGreaterThan(49);
    await page.clock.runFor(450);
    await expect(video).toHaveJSProperty('paused', false);
    // Seeking reveals the controls again; advance until the idle delay hides them.
    await expect
      .poll(async () => {
        await page.clock.runFor(500);
        return controls.evaluate(
          (element) => getComputedStyle(element).opacity,
        );
      })
      .toBe('0');
    const beforeReveal = await currentTime();
    await side(false);
    await expect(controls).toHaveCSS('opacity', '1');
    await page.clock.runFor(450);
    await expect(video).toHaveJSProperty('paused', false);
    expect(await currentTime()).toBeLessThan(beforeReveal + 3);
    await side(false);
    await page.clock.runFor(100);
    await expect(video).toHaveJSProperty('paused', false);
    await page.clock.runFor(250);
    await expect(video).toHaveJSProperty('paused', true);
    const beforeSeek = await currentTime();
    await side(false);
    await side(false);
    await side(false);
    await expect.poll(currentTime).toBeCloseTo(beforeSeek - 20, 0);
    await page.clock.runFor(450);
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
    // A pinned player in landscape keeps its controls on screen.
    await expect
      .poll(async () => (await player.boundingBox())!.height)
      .toBeCloseTo(390 * 0.6, 0);
    await expect(
      page.getByRole('button', { name: 'Fullscreen', exact: true }),
    ).toBeInViewport({ ratio: 1 });
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

  for (const mode of ['native', 'fallback'] as const) {
    for (const enteredOrientation of ['portrait', 'landscape'] as const) {
      test(`portrait rotation exits manual ${mode} fullscreen entered in ${enteredOrientation}`, async ({
        page,
      }, info) => {
        if (mode === 'fallback')
          await page.addInitScript(() => {
            HTMLElement.prototype.requestFullscreen = async () => {
              throw new DOMException(
                'Fullscreen unavailable',
                'NotSupportedError',
              );
            };
          });
        const cdp = await page.context().newCDPSession(page);
        const rotate = (landscape: boolean) =>
          cdp.send('Emulation.setDeviceMetricsOverride', {
            width: landscape ? 844 : 390,
            height: landscape ? 390 : 844,
            screenWidth: landscape ? 844 : 390,
            screenHeight: landscape ? 390 : 844,
            deviceScaleFactor: 1,
            mobile: true,
            screenOrientation: {
              type: landscape ? 'landscapePrimary' : 'portraitPrimary',
              angle: landscape ? 90 : 0,
            },
          });
        await rotate(false);
        const video = await openVideo(page);
        const player = page.getByRole('region', { name: 'Media player' });
        const enter = page.getByRole('button', {
          name: 'Fullscreen',
          exact: true,
        });
        const exit = page.getByRole('button', {
          name: 'Exit fullscreen',
          exact: true,
        });
        if (enteredOrientation === 'landscape') {
          await rotate(true);
          await exit.tap();
          await expect(enter).toBeVisible();
        }
        await enter.tap();
        await expect(exit).toBeVisible();
        await expect
          .poll(() => page.evaluate(() => document.fullscreenElement !== null))
          .toBe(mode === 'native');
        if (mode === 'fallback')
          await expect(player).toHaveJSProperty('popover', 'manual');
        if (enteredOrientation === 'portrait') await rotate(true);
        await expect(exit).toBeVisible();
        await expect
          .poll(async () => (await player.boundingBox())!.width)
          .toBe(844);
        await expect
          .poll(async () => (await player.boundingBox())!.height)
          .toBe(390);
        await page.screenshot({
          path: info.outputPath('manual-fullscreen-landscape.png'),
        });
        await rotate(false);
        await expect(enter).toBeVisible();
        expect(
          await page.evaluate(() => document.fullscreenElement),
        ).toBeNull();
        await expect(player).not.toHaveAttribute('popover');
        await expect
          .poll(async () => (await player.boundingBox())!.width)
          .toBe(390);
        await expect
          .poll(async () => (await player.boundingBox())!.height)
          .toBeCloseTo((390 * 9) / 16, 0);
        await expect.poll(async () => (await player.boundingBox())!.y).toBe(0);
        await expect(video).toHaveJSProperty('paused', false);
        await page.screenshot({
          path: info.outputPath('manual-fullscreen-portrait.png'),
        });
        await cdp.detach();
      });
    }
  }

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
    await expect(drawer).toHaveCSS('transform', 'none');
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
