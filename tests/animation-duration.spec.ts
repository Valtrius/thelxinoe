import { test, expect, type Page } from '@playwright/test';
import { responseGate } from './helpers/response-gate';
import { installUiFixture } from './helpers/ui-fixture';

test.use({ video: 'on' });
test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

const ready = async (page: Page) => {
  await expect(page.locator('[data-page-content]')).toHaveAttribute(
    'data-page-phase',
    'idle',
  );
};

for (const initialStatus of [200, 503]) {
  test(`a saved motion choice survives an initial ${initialStatus} response after sign-in`, async ({
    page,
  }, info) => {
    await installUiFixture(page, { signedIn: false });
    const gate = responseGate();
    const saved = {
      theme: 'dark',
      poster_columns: 9,
      animation_duration_ms: 200,
    };
    let initial = true;
    await page.route('**/api/v1/me/appearance', async (route) => {
      if (route.request().method() === 'PATCH') {
        Object.assign(saved, route.request().postDataJSON());
        return route.fulfill({ json: saved });
      }
      if (!initial) return route.fulfill({ json: saved });
      initial = false;
      const snapshot = { ...saved };
      await gate.promise;
      return route.fulfill({
        status: initialStatus,
        json:
          initialStatus === 200
            ? snapshot
            : { error: { message: 'Delayed preference load failed' } },
      });
    });
    try {
      await page.goto('/');
      await page.getByLabel('Username', { exact: true }).fill('fixture');
      await page
        .getByLabel('Password', { exact: true })
        .fill('test-only long passphrase');
      await page.getByRole('button', { name: 'Sign in', exact: true }).click();
      await page.getByRole('link', { name: 'Settings', exact: true }).click();
      const slider = page.getByRole('slider', { name: 'Animation duration' });
      const committed = page.waitForResponse(
        (response) =>
          response.url().endsWith('/me/appearance') &&
          response.request().method() === 'PATCH',
      );
      await slider.fill('0');
      await committed;
      await page.evaluate(() => new Promise(requestAnimationFrame));
      expect(saved.animation_duration_ms).toBe(0);
      gate.release();
      await ready(page);
      if (initialStatus === 200) {
        await expect(
          page.getByRole('group', { name: 'Poster density' }),
        ).toContainText('9 columns');
      } else
        await expect(
          page.getByText('Appearance preferences could not be loaded.').first(),
        ).toBeVisible();
      await expect(slider).toHaveValue('0');
      await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
      await page.screenshot({
        path: info.outputPath(
          `local-choice-after-initial-${initialStatus}.png`,
        ),
      });
      await info.attach('preference-load-race.json', {
        body: JSON.stringify({ initialStatus, saved }, null, 2),
        contentType: 'application/json',
      });
    } finally {
      gate.release();
      await page.unrouteAll({ behavior: 'wait' });
    }
  });
}

test('failed duration saves remain visible through initial loading and reconnect until that preference saves', async ({
  page,
}, info) => {
  const { user } = await (await page.request.get('/api/v1/auth/me')).json();
  await page.request.post('/api/v1/auth/logout', {
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  await page.addInitScript((id) => {
    localStorage.setItem(
      `thelxinoe::${id}:navigation`,
      JSON.stringify({ section: 'Settings', settingsSection: 'account' }),
    );
  }, user.id);
  const initial = responseGate();
  const reconnect = responseGate();
  const saved = {
    theme: 'dark',
    poster_columns: 9,
    animation_duration_ms: 200,
  };
  let reads = 0;
  await page.route('**/api/v1/me/appearance', async (route) => {
    if (route.request().method() === 'PATCH') {
      const change = route.request().postDataJSON();
      if (change.animation_duration_ms === 0)
        return route.fulfill({
          status: 503,
          json: { error: { message: 'Preference save unavailable' } },
        });
      Object.assign(saved, change);
      return route.fulfill({ json: saved });
    }
    const snapshot = { ...saved };
    reads++;
    if (reads === 1) await initial.promise;
    if (reads === 2) await reconnect.promise;
    return route.fulfill({ json: snapshot });
  });
  try {
    await page.goto('/');
    await page.getByLabel('Username', { exact: true }).fill('fixture');
    await page
      .getByLabel('Password', { exact: true })
      .fill('test-only long passphrase');
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    const slider = page.getByRole('slider', { name: 'Animation duration' });
    const error = page
      .getByText(
        'Your appearance changed here, but could not be saved to the server.',
      )
      .first();
    await slider.fill('0');
    await expect(error).toBeVisible();
    initial.release();
    await expect.poll(() => reads).toBe(2);
    await expect(
      page.getByRole('group', { name: 'Poster density' }),
    ).toContainText('9 columns');
    await expect(slider).toHaveValue('0');
    await expect(error).toBeVisible();
    const refreshed = page.waitForResponse(
      (response) =>
        response.url().endsWith('/me/appearance') &&
        response.request().method() === 'GET',
    );
    reconnect.release();
    await refreshed;
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(slider).toHaveValue('0');
    await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
    await expect(error).toBeVisible();
    await page.screenshot({
      path: info.outputPath('failed-save-after-reconnect.png'),
    });
    await page
      .getByRole('button', { name: 'Light theme', exact: true })
      .click();
    await expect.poll(() => saved.theme).toBe('light');
    await expect(error).toBeVisible();
    await slider.fill('137');
    await expect.poll(() => saved.animation_duration_ms).toBe(137);
    await expect(error).toHaveCount(0);
    await page.reload();
    await ready(page);
    await expect(slider).toHaveValue('137');
    await page.screenshot({
      path: info.outputPath('corrected-save-after-reload.png'),
    });
    await info.attach('failed-save-recovery.json', {
      body: JSON.stringify({ reads, saved }, null, 2),
      contentType: 'application/json',
    });
  } finally {
    initial.release();
    reconnect.release();
    await page.unrouteAll({ behavior: 'wait' });
  }
});

test('a delayed reconnect read cannot overwrite a newer successful duration save', async ({
  page,
}, info) => {
  const gate = responseGate();
  const saved = { animation_duration_ms: 200 };
  let reads = 0;
  await page.route('**/api/v1/me/appearance', async (route) => {
    if (route.request().method() === 'PATCH') {
      Object.assign(saved, route.request().postDataJSON());
      return route.fulfill({ json: saved });
    }
    const snapshot = { ...saved };
    if (++reads === 2) await gate.promise;
    return route.fulfill({ json: snapshot });
  });
  try {
    await page.goto('/#settings/account');
    await ready(page);
    await expect.poll(() => reads).toBe(2);
    const slider = page.getByRole('slider', { name: 'Animation duration' });
    const committed = page.waitForResponse(
      (response) =>
        response.url().endsWith('/me/appearance') &&
        response.request().method() === 'PATCH',
    );
    await slider.fill('137');
    await committed;
    await page.evaluate(() => new Promise(requestAnimationFrame));
    expect(saved.animation_duration_ms).toBe(137);
    const refreshed = page.waitForResponse(
      (response) =>
        response.url().endsWith('/me/appearance') &&
        response.request().method() === 'GET',
    );
    gate.release();
    await refreshed;
    await page.evaluate(() => new Promise(requestAnimationFrame));
    await expect(slider).toHaveValue('137');
    await expect(page.locator('html')).toHaveCSS(
      '--animation-duration',
      '137ms',
    );
    await page.screenshot({
      path: info.outputPath('saved-choice-after-stale-reconnect.png'),
    });
    await info.attach('stale-reconnect.json', {
      body: JSON.stringify({ reads, saved }, null, 2),
      contentType: 'application/json',
    });
  } finally {
    gate.release();
    await page.unrouteAll({ behavior: 'wait' });
  }
});

test('a native slider drag coalesces slow saves and persists its final choice before reload', async ({
  page,
}, info) => {
  await page.goto('/#settings/account');
  await ready(page);
  const gate = responseGate();
  const requests: number[] = [];
  await page.route('**/api/v1/me/appearance', async (route) => {
    const request = route.request();
    if (request.method() !== 'PATCH') return route.fallback();
    const value = request.postDataJSON().animation_duration_ms;
    if (value !== undefined) {
      requests.push(value);
      if (requests.length === 1) await gate.promise;
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
    return route.fallback();
  });
  try {
    const slider = page.getByRole('slider', { name: 'Animation duration' });
    const box = (await slider.boundingBox())!;
    await page.mouse.move(box.x + 8, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width - 8, box.y + box.height / 2, {
      steps: 60,
    });
    await page.mouse.up();
    await expect(slider).toHaveValue('500');
    await expect.poll(() => requests.length).toBe(1);
    await page.getByRole('button', { name: 'Dark theme', exact: true }).click();
    gate.release();
    await expect
      .poll(
        async () => {
          const saved = await (
            await page.request.get('/api/v1/me/appearance')
          ).json();
          return { duration: saved.animation_duration_ms, theme: saved.theme };
        },
        { timeout: 2500 },
      )
      .toEqual({ duration: 500, theme: 'dark' });
    expect(requests).toEqual([0, 500]);
    await page.reload();
    await ready(page);
    await expect(slider).toHaveValue('500');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await page.screenshot({
      path: info.outputPath('latest-drag-value-after-reload.png'),
    });
    await info.attach('coalesced-duration-saves.json', {
      body: JSON.stringify(requests),
      contentType: 'application/json',
    });
  } finally {
    gate.release();
    await page.unrouteAll({ behavior: 'wait' });
  }
});

for (const duration of [0, 500]) {
  test(`startup defers interface motion until the user's ${duration}ms preference loads`, async ({
    page,
    request,
  }, info) => {
    await request.patch('/api/v1/me/appearance', {
      headers: { 'X-Thelxinoe-Client': '1' },
      data: { animation_duration_ms: duration },
    });
    const gate = responseGate();
    let requested = false;
    await page.route('**/api/v1/me/appearance', async (route) => {
      requested = true;
      await gate.promise;
      await route.fallback();
    });
    try {
      await page.goto('/#settings/account');
      await expect.poll(() => requested).toBe(true);
      await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
      expect(
        await page.evaluate(
          () =>
            document
              .getAnimations()
              .filter(
                (a) =>
                  a.playState === 'running' &&
                  Number.isFinite(
                    Number(a.effect?.getComputedTiming().endTime),
                  ),
              ).length,
        ),
      ).toBe(0);
      await page.screenshot({
        path: info.outputPath('waiting-for-user-preferences.png'),
      });
      gate.release();
      await ready(page);
      await expect(
        page.getByRole('slider', { name: 'Animation duration' }),
      ).toHaveValue(String(duration));
      await expect(page.locator('html')).toHaveAttribute(
        'data-motion',
        duration ? 'on' : 'off',
      );
      await info.attach('loaded-user-duration.json', {
        body: JSON.stringify({
          duration,
          effective: await page.evaluate(() =>
            getComputedStyle(document.documentElement).getPropertyValue(
              '--animation-duration',
            ),
          ),
        }),
        contentType: 'application/json',
      });
    } finally {
      gate.release();
      await page.unrouteAll({ behavior: 'wait' });
    }
  });
}

test('a failed initial preference load keeps interface motion off until the user chooses a duration', async ({
  page,
}, info) => {
  await page.route('**/api/v1/me/appearance', async (route) => {
    if (route.request().method() !== 'GET') return route.fallback();
    await route.fulfill({
      status: 503,
      json: { error: { message: 'Preference fixture unavailable' } },
    });
  });
  await page.goto('/#settings/account');
  await ready(page);
  await expect(page.locator('html')).toHaveAttribute('data-motion', 'off');
  await expect(
    page.getByText('Appearance preferences could not be loaded.').first(),
  ).toBeVisible();
  await page.getByRole('slider', { name: 'Animation duration' }).fill('137');
  await expect(page.locator('html')).toHaveAttribute('data-motion', 'on');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .animation_duration_ms,
    )
    .toBe(137);
  await page.screenshot({
    path: info.outputPath('explicit-user-choice-after-load-failure.png'),
  });
});

for (const duration of [0, 137, 500]) {
  test(`animation duration ${duration}ms persists and controls CSS, layouts, navigation and drawers`, async ({
    page,
  }, info) => {
    await page.goto('/#settings/account');
    await ready(page);
    const slider = page.getByRole('slider', { name: 'Animation duration' });
    await expect(slider).toHaveValue('200');
    await slider.fill(String(duration));
    await expect(slider).toHaveValue(String(duration));
    await expect
      .poll(
        async () =>
          (await (await page.request.get('/api/v1/me/appearance')).json())
            .animation_duration_ms,
      )
      .toBe(duration);
    await page.reload();
    await ready(page);
    await expect(slider).toHaveValue(String(duration));
    await expect(slider).toHaveAttribute(
      'aria-valuetext',
      duration ? `${duration} ms` : 'No animation',
    );

    const evidence: unknown[] = [];
    // Record real browser animation creation without replacing its behavior.
    await page.evaluate(() => {
      const created: {
        target: string;
        duration: unknown;
        frames: PropertyIndexedKeyframes | Keyframe[] | null;
      }[] = [];
      Object.assign(window, { createdMotion: created });
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (frames, options) {
        const animation = animate.call(this, frames, options);
        created.push({
          target: this.tagName + '.' + this.classList[0],
          duration: animation.effect?.getTiming().duration,
          frames,
        });
        return animation;
      };
    });
    const capture = async (action: () => Promise<unknown>, label: string) => {
      await page.evaluate(() => {
        (
          window as unknown as { createdMotion: unknown[] }
        ).createdMotion.length = 0;
      });
      await action();
      await ready(page);
      const created = await page.evaluate(
        () =>
          (window as unknown as { createdMotion: { duration: number }[] })
            .createdMotion,
      );
      if (duration) {
        expect(created.length, label).toBeGreaterThan(0);
        expect(
          created.every((animation) => animation.duration === duration),
          label,
        ).toBe(true);
      } else expect(created, label).toEqual([]);
      evidence.push({ label, created });
    };
    await capture(
      () => page.getByRole('link', { name: 'Movies', exact: true }).click(),
      'page change',
    );
    const card = page.locator('.library-tile').first();
    await expect(card).toBeVisible();
    const css = await card.evaluate((element) => ({
      lift: getComputedStyle(element).transitionDuration,
      thumbnail: getComputedStyle(element.querySelector('img')!)
        .transitionDuration,
    }));
    expect(css.lift).toBe(`${duration / 1000}s`);
    expect(css.thumbnail).toBe(`${duration / 1000}s`);
    evidence.push({ css });
    await capture(
      () =>
        page
          .getByRole('button', { name: 'Collapse sidebar', exact: true })
          .click(),
      'sidebar',
    );
    await capture(async () => {
      await card.hover();
      await page.keyboard.down('Control');
      await page.mouse.wheel(0, -100);
      await page.keyboard.up('Control');
      await expect
        .poll(
          async () =>
            (await (await page.request.get('/api/v1/me/appearance')).json())
              .poster_columns,
        )
        .toBe(5);
    }, 'grid density');
    await page.setViewportSize({ width: 390, height: 844 });
    await capture(
      () =>
        page
          .getByRole('button', { name: 'Open navigation', exact: true })
          .click(),
      'mobile drawer',
    );
    await page.keyboard.press('Escape');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await page.screenshot({
      path: info.outputPath(`motion-${duration}ms.png`),
    });
    await info.attach('animation-timings.json', {
      body: JSON.stringify(evidence, null, 2),
      contentType: 'application/json',
    });
  });
}

for (const duration of [0, 500]) {
  test(`controls and shelf scrolling follow ${duration}ms while activity keeps its normal timing`, async ({
    page,
  }, info) => {
    await page.goto('/#settings/account');
    await ready(page);
    await page
      .getByRole('slider', { name: 'Animation duration' })
      .fill(String(duration));
    const choice = page.getByRole('button', {
      name: 'Dark theme',
      exact: true,
    });
    await page.evaluate(() => {
      const durations: unknown[] = [];
      Object.assign(window, { choiceDurations: durations });
      const animate = Element.prototype.animate;
      Element.prototype.animate = function (frames, options) {
        const animation = animate.call(this, frames, options);
        if (this.hasAttribute('data-choice-selection'))
          durations.push(animation.effect?.getTiming().duration);
        return animation;
      };
    });
    await choice.click();
    const selection = page.locator('[data-choice-selection]').last();
    const settings = await selection.evaluate((element) => ({
      durations: (window as unknown as { choiceDurations: unknown[] })
        .choiceDurations,
      button: getComputedStyle(element.parentElement!.querySelector('button')!)
        .transitionDuration,
      switch: getComputedStyle(
        document.querySelector('.ui-switch .transition-colors')!,
      ).transitionDuration,
    }));
    expect(settings.button).toBe(`${duration / 1000}s`);
    expect(settings.switch).toBe(`${duration / 1000}s`);
    if (duration) expect(settings.durations).toContain(duration);
    else expect(settings.durations).toEqual([]);

    await page.getByRole('link', { name: 'Home', exact: true }).click();
    await ready(page);
    const shelf = page.getByRole('region', {
      name: 'Continue watching',
      exact: true,
    });
    const frames = await shelf.evaluate(async (element) => {
      const rail = element.querySelector<HTMLElement>('.media-row-rail')!;
      const before = performance.now();
      (
        element.querySelector(
          '[aria-label="Next Continue watching"]',
        ) as HTMLElement
      ).click();
      const frames = [{ time: 0, left: rail.scrollLeft }];
      while (performance.now() - before < 650) {
        await new Promise(requestAnimationFrame);
        frames.push({
          time: performance.now() - before,
          left: rail.scrollLeft,
        });
      }
      return frames;
    });
    expect(frames.at(-1)!.left).toBeGreaterThan(0);
    if (duration) {
      expect(frames[0].left).toBe(0);
      expect(
        frames.some(
          (frame) =>
            frame.time < 400 &&
            frame.left > 0 &&
            frame.left < frames.at(-1)!.left,
        ),
      ).toBe(true);
      expect(
        frames
          .filter((frame) => frame.time >= 550)
          .every((frame) => frame.left === frames.at(-1)!.left),
      ).toBe(true);
    } else
      expect(frames.every((frame) => frame.left === frames.at(-1)!.left)).toBe(
        true,
      );

    let release!: () => void;
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route('**/api/v1/me/home', async (route) => {
      await held;
      await route.fallback();
    });
    try {
      const refresh = page.getByRole('button', {
        name: 'Refresh Home',
        exact: true,
      });
      await refresh.click();
      const spinner = refresh.locator('.animate-spin');
      await expect(spinner).toBeAttached();
      const activity = await spinner.evaluate((element) => ({
        duration: getComputedStyle(element).animationDuration,
        name: getComputedStyle(element).animationName,
      }));
      expect(activity.duration).toBe('1s');
      expect(activity.name).toBe('spin');
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await expect(spinner).toHaveCSS('animation-name', 'none');
      await page.emulateMedia({ reducedMotion: 'no-preference' });
      await expect(spinner).toHaveCSS('animation-name', 'spin');
      await info.attach('controls-scrolling-and-activity.json', {
        body: JSON.stringify({ settings, frames, activity }, null, 2),
        contentType: 'application/json',
      });
    } finally {
      release();
      await page.unrouteAll({ behavior: 'wait' });
    }
    await ready(page);
    await page.screenshot({
      path: info.outputPath(`activity-${duration}ms.png`),
    });
  });
}

test('progress and activity indicators retain their own timing at zero and respect reduced motion', async ({
  page,
}, info) => {
  await installUiFixture(page, { role: 'admin', settingsSection: 'account' });
  const item = {
    id: 'ffmpeg',
    policy: 'inherit',
    effective_policy: 'notify',
    channel: 'stable',
    pinned: false,
    revision: 0,
    installed: null,
    previous: null,
    candidate: null,
    checked_at: Math.floor(Date.now() / 1000),
    check_error: null,
    integrity_error: null,
    job: {
      id: 'ffmpeg-download',
      action: 'bootstrap',
      manual: true,
      stage: 'downloading',
      received: 20,
      total: 100,
      reason: null,
      error: null,
    },
  };
  await page.route('**/api/v1/admin/tools', (route) =>
    route.fulfill({ json: { items: [item], supported: true } }),
  );
  await page.goto('/');
  await ready(page);
  await page.getByRole('slider', { name: 'Animation duration' }).fill('0');
  await page.getByRole('link', { name: 'Server', exact: true }).click();
  await ready(page);
  const tools = page.getByRole('region', { name: 'Server tools', exact: true });
  const circle = tools.locator('circle[pathLength="100"]');
  await expect(circle).toHaveCSS('transition-duration', '0.3s');
  item.job.received = 80;
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  await expect(circle).toHaveAttribute('stroke-dasharray', '80 100');
  const progress = await circle.evaluate((element) =>
    element.getAnimations().map((a) => ({
      duration: a.effect?.getTiming().duration,
      state: a.playState,
    })),
  );
  expect(
    progress.some((a) => a.duration === 300 && a.state === 'running'),
  ).toBe(true);
  item.job.stage = 'verifying';
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  await expect(circle).toHaveCSS('animation-duration', '1.25s');
  await expect(circle).toHaveCSS('animation-name', 'spin');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(circle).toHaveCSS('animation-name', 'none');
  await expect(circle).toHaveCSS('transition-duration', '0s');
  await info.attach('progress-independent-of-ui-timing.json', {
    body: JSON.stringify(progress, null, 2),
    contentType: 'application/json',
  });
  await page.screenshot({
    path: info.outputPath('reduced-motion-progress.png'),
  });
});

test('reduced motion overrides the saved duration and live changes settle active motion', async ({
  page,
}, info) => {
  await page.goto('/#settings/account');
  await ready(page);
  const slider = page.getByRole('slider', { name: 'Animation duration' });
  await slider.fill('500');
  await expect
    .poll(
      async () =>
        (await (await page.request.get('/api/v1/me/appearance')).json())
          .animation_duration_ms,
    )
    .toBe(500);
  await page
    .getByRole('button', { name: 'Collapse sidebar', exact: true })
    .click();
  await slider.fill('0');
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.getAnimations().filter((a) => a.playState === 'running')
            .length,
      ),
    )
    .toBe(0);
  await slider.fill('500');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page
    .getByRole('button', { name: 'Expand sidebar', exact: true })
    .click();
  await page.getByRole('link', { name: 'Movies', exact: true }).click();
  await ready(page);
  expect(
    await page.evaluate(
      () =>
        document.getAnimations().filter((a) => a.playState === 'running')
          .length,
    ),
  ).toBe(0);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  await ready(page);
  await expect(slider).toHaveValue('500');
  await page.screenshot({
    path: info.outputPath('reduced-motion-preference-preserved.png'),
  });
});
