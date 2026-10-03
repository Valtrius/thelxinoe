import { test, expect, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import { waitForViewportFit } from '../scripts/ci-readiness.mjs';

async function selectRadarr(page: Page) {
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'Radarr', exact: true })
    .click();
}

test('installation progress polls automatically without queued banners across tabs', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  let installed = false;
  let phase = 'queued';
  let stackReads = 0;
  await page.route('**/api/v1/admin/stack/install', async (route) => {
    installed = true;
    await route.fulfill({ json: { id: 'managed-sonarr', state: 'queued' } });
  });
  await page.route('**/api/v1/admin/stack', async (route) => {
    stackReads++;
    await route.fulfill({
      json: {
        items: [],
        provisions: installed
          ? [{ id: 'managed-sonarr', kind: 'sonarr', state: phase }]
          : [],
      },
    });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Sonarr', exact: true }).click();
  await page
    .getByRole('button', { name: 'Install Sonarr', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Sonarr', exact: true }),
  ).toContainText('Installing');
  await expect(page.getByText(/installation queued/i)).toHaveCount(0);
  await page.getByRole('button', { name: 'Radarr', exact: true }).click();
  await expect(page.locator('.service-workspace > .notice')).toHaveCount(0);
  phase = 'complete';
  const before = stackReads;
  await expect
    .poll(() => stackReads, { timeout: 7000 })
    .toBeGreaterThan(before);
  await expect(
    page.getByRole('button', { name: 'Sonarr', exact: true }),
  ).not.toContainText('Installing');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('a stopped service can be removed after reviewing its configuration cleanup', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  let stopped = false;
  let removed = false;
  const actions: unknown[] = [];
  await page.route('**/api/v1/admin/stack', async (route) => {
    await route.fulfill({
      json: {
        items: removed
          ? []
          : [
              {
                id: 'managed-radarr',
                kind: 'radarr',
                phase: 'active',
                running: !stopped,
                existence: 'present',
                drift: false,
                can_remove: stopped,
              },
            ],
        provisions: removed
          ? []
          : [{ id: 'managed-radarr', kind: 'radarr', state: 'complete' }],
      },
    });
  });
  await page.route('**/api/v1/admin/managers', async (route) => {
    if (removed) await route.fulfill({ json: { items: [] } });
    else await route.fallback();
  });
  await page.route(
    '**/api/v1/admin/stack/managed-radarr/action',
    async (route) => {
      actions.push(route.request().postDataJSON());
      removed = true;
      await route.fulfill({ json: { removed: true } });
    },
  );
  await page.goto('/');
  await selectRadarr(page);
  await expect(
    page.getByRole('button', { name: 'Radarr', exact: true }),
  ).toContainText('Running');
  await expect(
    page.getByRole('button', { name: 'Remove service', exact: true }),
  ).toHaveCount(0);
  stopped = true;
  await expect(
    page.getByRole('button', { name: 'Radarr', exact: true }),
  ).toContainText('Stopped', { timeout: 7000 });
  await page
    .getByRole('button', { name: 'Remove service', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Remove Radarr?' });
  await expect(dialog).toContainText('Media files and other services are kept');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(actions).toEqual([]);
  await page
    .getByRole('button', { name: 'Remove service', exact: true })
    .click();
  await dialog
    .getByRole('button', { name: 'Remove service and configuration' })
    .click();
  await expect(
    page.getByRole('button', { name: 'Install Radarr', exact: true }),
  ).toBeVisible();
  expect(actions).toEqual([{ action: 'remove' }]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('removal and installation keep independent locks, feedback and polling', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  let finishRemoval: (() => void) | undefined;
  let finishInstall: (() => void) | undefined;
  let stackReads = 0;
  let retiring = false;
  await page.route('**/api/v1/admin/stack', async (route) => {
    stackReads++;
    await route.fulfill({
      json: {
        items: [
          {
            id: 'managed-radarr',
            kind: 'radarr',
            phase: 'active',
            running: false,
            existence: 'present',
            drift: false,
            can_remove: true,
          },
        ],
        provisions: [
          {
            id: 'managed-radarr',
            kind: 'radarr',
            state: retiring ? 'retiring' : 'complete',
          },
        ],
      },
    });
  });
  await page.route(
    '**/api/v1/admin/stack/managed-radarr/action',
    async (route) => {
      retiring = true;
      await new Promise<void>((resolve) => {
        finishRemoval = resolve;
      });
      await route.fulfill({
        status: 409,
        json: { error: { message: 'Removal needs attention' } },
      });
    },
  );
  await page.route('**/api/v1/admin/stack/install', async (route) => {
    expect(route.request().postDataJSON().kind).toBe('sonarr');
    await new Promise<void>((resolve) => {
      finishInstall = resolve;
    });
    await route.fulfill({ json: { id: 'managed-sonarr', state: 'queued' } });
  });
  await page.goto('/');
  await selectRadarr(page);
  await page
    .getByRole('button', { name: 'Remove service', exact: true })
    .click();
  await page
    .getByRole('button', { name: 'Remove service and configuration' })
    .click();
  await expect(
    page.getByRole('dialog', { name: 'Remove Radarr?' }),
  ).not.toBeVisible();
  await expect.poll(() => !!finishRemoval).toBe(true);
  await expect(
    page.getByRole('button', { name: 'Start', exact: true }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Sonarr', exact: true }).click();
  await page
    .getByRole('button', { name: 'Install Sonarr', exact: true })
    .click();
  await expect.poll(() => !!finishInstall).toBe(true);
  await expect(
    page.getByRole('button', { name: 'Install Sonarr', exact: true }),
  ).toBeDisabled();
  await page.getByRole('button', { name: 'Lidarr', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Install Lidarr', exact: true }),
  ).toBeEnabled();
  const reads = stackReads;
  await expect.poll(() => stackReads, { timeout: 7000 }).toBeGreaterThan(reads);
  finishRemoval!();
  await page.getByRole('button', { name: 'Radarr', exact: true }).click();
  await expect(page.locator('.rail-identity:not(.inactive)')).toContainText(
    'Removal needs attention',
  );
  await expect(
    page.getByRole('button', { name: 'Remove service', exact: true }),
  ).toBeEnabled();
  await page.getByRole('button', { name: 'Sonarr', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Install Sonarr', exact: true }),
  ).toBeDisabled();
  await expect(page.locator('.rail-identity:not(.inactive)')).not.toContainText(
    'Removal needs attention',
  );
  finishInstall!();
  await expect(
    page.getByRole('button', { name: 'Install Sonarr', exact: true }),
  ).toBeEnabled();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('an older update poll cannot unlock a newly queued update', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
    stackCapabilities: {
      can_retire: true,
      can_remove: true,
      can_recreate: true,
    },
  });
  let reads = 0;
  let queued = false;
  let finishOldPoll: (() => void) | undefined;
  let finishSubmission: (() => void) | undefined;
  await page.route('**/api/v1/admin/service-updates', async (route) => {
    reads++;
    const items = queued
      ? [{ id: 'update-radarr', service_id: 'managed-radarr', state: 'queued' }]
      : [];
    if (reads === 2)
      await new Promise<void>((resolve) => {
        finishOldPoll = resolve;
      });
    await route.fulfill({
      json: {
        policies: [
          {
            service_id: 'managed-radarr',
            policy: 'notify',
            window_start: 3,
            window_end: 5,
            candidate: 'ghcr.io/example/radarr:new',
            checked_at: 1700000000,
            error: null,
          },
        ],
        timezone: 'UTC',
        items,
        services: [{ id: 'managed-radarr', kind: 'radarr' }],
        server_policy: { policy: 'notify', window_start: 3, window_end: 5 },
      },
    });
  });
  await page.route(
    '**/api/v1/admin/service-updates/preflight/managed-radarr',
    async (route) => {
      await new Promise<void>((resolve) => {
        finishSubmission = resolve;
      });
      queued = true;
      await route.fulfill({ json: { id: 'update-radarr', state: 'queued' } });
    },
  );
  await page.goto('/');
  await selectRadarr(page);
  const check = page.getByRole('button', {
    name: 'Check compatibility',
    exact: true,
  });
  await expect(check).toBeEnabled();
  await expect.poll(() => !!finishOldPoll, { timeout: 7000 }).toBe(true);
  await check.click();
  await expect.poll(() => !!finishSubmission).toBe(true);
  const controls = page.getByRole('complementary', {
    name: 'Service controls',
  });
  await expect(controls.getByRole('button')).toHaveCount(0);
  await expect(
    page.getByText('Retiring removes the installation', { exact: false }),
  ).toHaveCount(0);
  finishSubmission!();
  await expect.poll(() => reads).toBe(3);
  await expect(check).toBeDisabled();
  await expect(controls.getByRole('button')).toHaveCount(0);
  const staleResponse = page.waitForResponse((response) =>
    response.url().endsWith('/admin/service-updates'),
  );
  finishOldPoll!();
  await staleResponse;
  await page.getByRole('button', { name: 'Sonarr', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Install Sonarr', exact: true }),
  ).toBeEnabled();
  await page.getByRole('button', { name: 'Radarr', exact: true }).click();
  await expect(check).toBeDisabled();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const state of [
  'queued',
  'submitting',
  'preparing',
  'snapshotting',
  'preflight',
  'queued-activate',
  'recovery-snapshot',
  'isolated-live-validation',
  'activating',
  'queued-recover',
]) {
  test(`a ${state} update hides only its own service controls`, async ({
    page,
  }, testInfo) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
      serviceUpdateState: state,
      stackCapabilities: {
        can_retire: true,
        can_remove: true,
        can_recreate: true,
      },
    });
    await page.goto('/');
    await selectRadarr(page);
    await expect(
      page
        .getByRole('complementary', { name: 'Service controls' })
        .getByRole('button'),
    ).toHaveCount(0);
    await expect(
      page.getByText('Retiring removes the installation', { exact: false }),
    ).toHaveCount(0);
    await expect(
      page.getByRole('button', { name: 'Check compatibility', exact: true }),
    ).toBeDisabled();
    if (state === 'activating' || state === 'preflight')
      await testInfo.attach(`service-controls-${state}`, {
        body: await page.screenshot(),
        contentType: 'image/png',
      });
    await page.getByRole('button', { name: 'Sonarr', exact: true }).click();
    await expect(
      page.getByRole('button', { name: 'Install Sonarr', exact: true }),
    ).toBeEnabled();
    await page.getByRole('button', { name: 'NZBGet', exact: true }).click();
    await expect(
      page.getByRole('button', { name: 'Pause all', exact: true }),
    ).toBeEnabled();
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

for (const savedControllerData of [false, true]) {
  test(`fresh service setup ${savedControllerData ? 'explains unmatched controller data' : 'offers installation for every service'}`, async ({
    page,
  }) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
    });
    for (const endpoint of ['managers', 'support']) {
      await page.route(`**/api/v1/admin/${endpoint}`, (route) =>
        route.fulfill({ json: { items: [] } }),
      );
    }
    await page.route('**/api/v1/admin/stack', (route) =>
      route.fulfill({
        json: {
          items: savedControllerData
            ? [
                {
                  id: 'old-radarr',
                  kind: 'radarr',
                  phase: 'active',
                  status: 'missing',
                  existence: 'missing',
                  registered: false,
                  can_recreate: false,
                  can_retire: false,
                  can_remove: false,
                },
              ]
            : [],
          provisions: [],
        },
      }),
    );
    await page.goto('/');
    const services = page.getByRole('navigation', { name: 'Select service' });
    if (savedControllerData) {
      await selectRadarr(page);
      await expect(
        services.getByRole('button', { name: 'Radarr', exact: true }),
      ).toContainText('Setup mismatch');
      await expect(
        page.getByRole('status', { name: 'Service setup mismatch' }),
      ).toContainText('no matching record');
      await expect(
        page.getByRole('button', { name: 'Recreate and start', exact: true }),
      ).toHaveCount(0);
      await expect(
        page.getByRole('button', { name: 'Restart', exact: true }),
      ).toHaveCount(0);
    } else {
      for (const name of [
        'Radarr',
        'Sonarr',
        'Lidarr',
        'Bazarr',
        'Prowlarr',
        'NZBGet',
      ]) {
        await services.getByRole('button', { name, exact: true }).click();
        await expect(
          services.getByRole('button', { name, exact: true }),
        ).toContainText('Not connected');
        await expect(
          page.getByRole('button', { name: `Install ${name}`, exact: true }),
        ).toBeEnabled();
      }
    }
    await expect(services).not.toContainText('Container missing');
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('profile pictures are cropped, resized, saved and removable', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  let saved: string | null = null;
  await page.route('**/api/v1/me/avatar', async (route) => {
    saved = route.request().postDataJSON().image;
    await route.fulfill({ json: { avatar: saved } });
  });
  await page.goto('/');
  await page.screenshot({
    path: 'test-results/server-settings-review.png',
    fullPage: true,
  });
  const data = await page.evaluate(() => {
    const canvas = document.createElement('canvas');
    canvas.width = 512;
    canvas.height = 256;
    const context = canvas.getContext('2d')!;
    context.fillStyle = '#ff0000';
    context.fillRect(0, 0, 256, 256);
    context.fillStyle = '#0000ff';
    context.fillRect(256, 0, 256, 256);
    return canvas.toDataURL('image/png').split(',')[1];
  });
  await expect(page.getByText('Choose a picture', { exact: true })).toHaveCount(
    0,
  );
  const pictureInput = page.getByLabel('Change profile picture');
  const avatar = pictureInput.locator('..');
  const editOverlay = avatar.locator('span[aria-hidden="true"]');
  await expect(editOverlay).toHaveCSS('opacity', '0');
  await avatar.hover();
  await expect(editOverlay).toHaveCSS('opacity', '1');
  await pictureInput.setInputFiles({
    name: 'two-colors.png',
    mimeType: 'image/png',
    buffer: Buffer.from(data, 'base64'),
  });
  const preview = page.getByRole('button', { name: 'Reframe profile picture' });
  await expect(preview).toBeVisible();
  await page
    .getByLabel('Picture zoom')
    .evaluate((element: HTMLInputElement) => {
      element.value = '2';
      element.dispatchEvent(new Event('input', { bubbles: true }));
    });
  await preview.scrollIntoViewIfNeeded();
  const bounds = (await preview.boundingBox())!;
  await page.mouse.move(bounds.x + 128, bounds.y + 128);
  await page.mouse.down();
  await page.mouse.move(bounds.x - 150, bounds.y + 128, { steps: 5 });
  await page.mouse.up();
  await page.getByLabel('Saved picture size').selectOption('128');
  await page.getByRole('button', { name: 'Save picture', exact: true }).click();
  await expect(
    page.getByRole('status').filter({ hasText: 'Profile picture saved' }),
  ).toBeVisible();
  const crop = await page.evaluate(async (source) => {
    const image = new Image();
    image.src = source!;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 128;
    const context = canvas.getContext('2d')!;
    context.drawImage(image, 0, 0);
    return {
      width: image.naturalWidth,
      height: image.naturalHeight,
      pixel: [...context.getImageData(64, 64, 1, 1).data],
    };
  }, saved);
  expect(crop.width).toBe(128);
  expect(crop.height).toBe(128);
  expect(crop.pixel[2]).toBeGreaterThan(230);
  expect(crop.pixel[0]).toBeLessThan(20);
  await expect(page.locator('.sidebar-profile .avatar img')).toHaveAttribute(
    'src',
    saved!,
  );
  await page.getByRole('button', { name: 'Remove picture' }).click();
  await expect(page.locator('.sidebar-profile .avatar img')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

async function expectContainedNavigation(page: Page) {
  await expect
    .poll(() =>
      page.locator('.settings-navigation').evaluate((nav) => {
        const workspace = nav
          .closest('.workspace-scroll')!
          .getBoundingClientRect();
        const bounds = nav.getBoundingClientRect();
        return Math.max(
          workspace.top - bounds.top,
          bounds.bottom - workspace.bottom,
        );
      }),
    )
    .toBeLessThanOrEqual(1);
}

test('adaptive account panels remain bounded while navigation follows resizing', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  await expect(
    page.getByRole('region', { name: 'Your display preferences', exact: true }),
  ).toBeVisible();
  await expectContainedNavigation(page);
  const panels = await page.locator('.settings-panels').evaluate((element) => ({
    width: element.getBoundingClientRect().width,
    inset:
      element.getBoundingClientRect().left -
      element.parentElement!.getBoundingClientRect().left,
  }));
  expect(panels.width).toBeGreaterThan(880);
  expect(panels.width).toBeLessThanOrEqual(1280);
  expect(panels.inset).toBe(0);
  await expect(
    page.getByText('Use the server default or choose your own timezone', {
      exact: false,
    }),
  ).toHaveCount(0);
  await expect(page.locator('.settings-panels > .panel').first()).toHaveCSS(
    'padding',
    '20px 0px 0px',
  );
  for (const viewport of [
    { width: 960, height: 800 },
    { width: 1440, height: 1200 },
  ]) {
    await page.setViewportSize(viewport);
    await expectContainedNavigation(page);
    await expect(page.locator('.settings-navigation')).toHaveCSS(
      'width',
      '196px',
    );
  }
  const theme = page
    .getByRole('region', { name: 'Your display preferences', exact: true })
    .getByRole('group', { name: 'Theme', exact: true });
  await expect(
    page.getByRole('group', { name: 'Theme', exact: true }),
  ).toHaveCount(1);
  await theme.getByRole('button', { name: 'Dark theme', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expectContainedNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('long settings content scrolls while navigation stays reachable', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { settingsSection: 'devices' });
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Your devices', exact: true }),
  ).toBeVisible();
  await page.locator('.workspace-scroll').evaluate((element) => {
    element.scrollTop = 450;
  });
  await expect
    .poll(() =>
      page
        .locator('.workspace-scroll')
        .evaluate((element) => element.scrollTop),
    )
    .toBe(450);
  await expectContainedNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('long admin navigation scrolls independently and keeps its last item reachable', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 560 });
  const fixture = await installUiFixture(page, { role: 'admin' });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  const last = nav.getByRole('button', { name: 'Audit', exact: true });
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
  expect(await nav.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  expect(
    await page
      .locator('.workspace-scroll')
      .evaluate((element) => element.scrollTop),
  ).toBe(0);
  await expectContainedNavigation(page);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('server settings contain display defaults and the version update icon', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  await page.goto('/');
  await page.screenshot({
    path: 'test-results/media-services-review.png',
    fullPage: true,
  });
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await expect(
    nav.getByRole('button', { name: 'Server updates', exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('combobox', {
      name: 'Server default timezone',
      exact: true,
    }),
  ).toHaveValue('UTC');
  await expect(
    page.getByRole('combobox', {
      name: 'Server default time format',
      exact: true,
    }),
  ).toHaveValue('24h');
  await expect(
    page.getByRole('heading', { name: 'Server updates', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Check for server updates', exact: true }),
  ).toBeVisible();
  for (const text of [
    'Controls server maintenance windows and server activity times.',
    'Server, controller and web share one release.',
    'Installed version 0.1.0',
    'Configure a release channel and its signing public key',
    'Automatic updates wait for idle playback and background work',
    'I understand that installation briefly stops the server.',
  ]) {
    await expect(page.getByText(text, { exact: false })).toHaveCount(0);
  }
  await page
    .getByRole('combobox', {
      name: 'Server default time format',
      exact: true,
    })
    .selectOption('12h');
  await expect
    .poll(() => fixture.writes)
    .toContainEqual({
      path: '/admin/settings',
      method: 'PUT',
      body: { timezone: 'UTC', time_format: '12h' },
    });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('all eight media services stay accessible without horizontal scrolling', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.goto('/');
  const nav = page.getByRole('navigation', { name: 'Select service' });
  const tabs = nav.getByRole('button');
  await expect(tabs).toHaveCount(8);
  for (const width of [1440, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const geometry = await tabs.evaluateAll((nodes) =>
      nodes.map((node) => {
        const rect = node.getBoundingClientRect();
        return { top: rect.top, width: rect.width, content: node.scrollWidth };
      }),
    );
    expect(new Set(geometry.map((tab) => tab.top)).size).toBeLessThanOrEqual(2);
    expect(geometry.every((tab) => tab.content <= tab.width + 1)).toBe(true);
    if (width === 390) {
      expect(
        await nav.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
      await tabs.last().click();
      await expect(
        page.getByRole('article', { name: 'NZBGet service' }),
      ).toBeVisible();
    }
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('media services preserve real controls across adaptive layouts', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.goto('/');
  await selectRadarr(page);
  const nav = page.getByRole('navigation', { name: 'Select service' });
  const radarr = page.getByRole('article', { name: 'Radarr service' });
  await expect(
    radarr.locator('.rail-identity:not(.inactive) > .service-status'),
  ).toHaveText('Running');
  await expect(
    radarr.getByRole('link', { name: /Open Radarr/ }),
  ).toHaveAttribute('href', '/services/radarr');
  await expect(radarr.getByText('6.0.0', { exact: true })).toBeVisible();
  await expect(
    radarr.getByRole('switch', {
      name: 'Monitor and search requests',
    }),
  ).toBeVisible();
  await expect(radarr.getByText('Service access', { exact: true })).toHaveCount(
    0,
  );
  await radarr
    .getByRole('group', { name: 'Update policy', exact: true })
    .getByRole('button', { name: 'Notify', exact: true })
    .click();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/admin/service-updates/policy/managed-radarr' &&
          JSON.stringify(write.body) ===
            '{"policy":"notify","window_start":3,"window_end":5}',
      ),
    )
    .toBe(true);
  for (const name of [
    'Seerr',
    'Sonarr',
    'Lidarr',
    'Bazarr',
    'Prowlarr',
    'NZBGet',
  ]) {
    await nav.getByRole('button', { name, exact: true }).click();
    await expect(
      page.getByRole('article', { name: `${name} service` }),
    ).toBeVisible();
    await expect(page.getByRole('article')).toHaveCount(1);
    await expect(
      page.getByRole('complementary', { name: 'Service controls' }),
    ).toBeVisible();
  }
  await nav.getByRole('button', { name: 'Sonarr', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Install Sonarr', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('form', { name: 'Connect existing Sonarr' }),
  ).toBeVisible();
  await nav.getByRole('button', { name: 'Prowlarr', exact: true }).click();
  await expect(
    page.getByRole('switch', { name: 'Enable Fixture indexer' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Review ownership transfer' }).click();
  await expect(
    page.getByRole('region', { name: 'Ownership review' }),
  ).toContainText('stops the original container');
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toBeDisabled();
  await page
    .getByRole('switch', {
      name: 'The previous Compose definition is disabled',
    })
    .press('Space');
  await page
    .getByRole('button', { name: 'Take ownership', exact: true })
    .click();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/admin/stack/adopt' &&
          JSON.stringify(write.body) ===
            '{"service_id":"support-prowlarr","review_id":"review-prowlarr","released_compose":true}',
      ),
    )
    .toBe(true);
  for (const width of [720, 480, 440, 320]) {
    await page.setViewportSize({ width, height: 850 });
    await page.screenshot({
      path: `.local/services-mobile-${width}.png`,
      fullPage: true,
    });
    await nav.getByRole('button', { name: 'Sonarr', exact: true }).click();
    const rail = await page.locator('.service-rail').boundingBox();
    const workspace = await page
      .getByRole('form', { name: 'Connect existing Sonarr' })
      .boundingBox();
    expect(workspace!.x).toBeGreaterThanOrEqual(rail!.x);
    expect(workspace!.y).toBeGreaterThanOrEqual(rail!.y + rail!.height);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('NZBGet combines active downloads with scrollable history and compact actions', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'NZBGet', exact: true })
    .click();
  const region = page.getByRole('region', {
    name: 'Current and recent downloads',
  });
  await expect(region.locator('tbody tr')).toHaveCount(33);
  await expect(region.locator('tbody tr').first()).toContainText(
    'Active fixture',
  );
  await expect(region.locator('tbody tr').first()).toContainText(
    /6 GiB\s*75%\s*Remaining\s*2 GiB/,
  );
  await expect(
    region.getByRole('img', { name: /Failed: failure .* unpack/ }),
  ).toBeVisible();
  await expect(
    region.getByRole('progressbar', {
      name: 'Finished fixture 2 download progress',
    }),
  ).toHaveAttribute('aria-valuetext', 'Progress unavailable');
  expect(
    await region.evaluate(
      (node) =>
        node.scrollHeight > node.clientHeight && node.clientHeight < 560,
    ),
  ).toBe(true);
  await page
    .getByRole('button', { name: 'Pause Active fixture', exact: true })
    .click();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/admin/support/support-nzbget' &&
          JSON.stringify(write.body) === '{"action":"pause","item_id":1}',
      ),
    )
    .toBe(true);
  await page
    .getByRole('button', { name: 'Resume Paused fixture', exact: true })
    .click();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          JSON.stringify(write.body) === '{"action":"resume","item_id":2}',
      ),
    )
    .toBe(true);
  await page
    .getByRole('button', { name: 'Remove Active fixture', exact: true })
    .click();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          JSON.stringify(write.body) === '{"action":"remove","item_id":1}',
      ),
    )
    .toBe(true);
  await page.screenshot({
    path: '.local/services-desktop.png',
    fullPage: true,
  });
  for (const width of [720, 480, 440, 320]) {
    await page.setViewportSize({ width, height: 850 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(
      await region.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
    if (width <= 440) {
      const row = region.locator('tbody tr').first();
      await expect(row).toHaveCSS('display', 'grid');
      expect((await row.boundingBox())!.height).toBeLessThanOrEqual(66);
    }
  }
  await region.scrollIntoViewIfNeeded();
  await page.screenshot({ path: '.local/services-mobile.png', fullPage: true });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('NZBGet URL has compact login and password copy buttons', async ({
  page,
}) => {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  await page.goto('/');
  const services = page.getByRole('navigation', { name: 'Select service' });
  await services.getByRole('button', { name: 'NZBGet', exact: true }).click();
  const line = page.locator('.rail-identity:not(.inactive) .service-url-line');
  const url = line.getByRole('link', { name: 'Open NZBGet', exact: true });
  const copyLogin = line.getByRole('button', { name: 'Copy NZBGet login' });
  const copyPassword = line.getByRole('button', {
    name: 'Copy NZBGet password',
  });
  await expect(url).toBeVisible();
  await expect(url).toHaveAttribute('href', '/services/nzbget');
  await expect(copyLogin).toBeVisible();
  await expect(copyPassword).toBeVisible();
  await expect(page.getByRole('region', { name: 'NZBGet login' })).toHaveCount(
    0,
  );
  expect(
    fixture.writes.some(
      (write) => write.path === '/admin/support/support-nzbget/login',
    ),
  ).toBe(false);
  await page.setViewportSize({ width: 320, height: 850 });
  await waitForViewportFit(page);
  const urlBox = await url.boundingBox();
  const loginBox = await copyLogin.boundingBox();
  const passwordBox = await copyPassword.boundingBox();
  await line.screenshot({ path: '.local/nzbget-url-line-320.png' });
  expect(urlBox && loginBox && passwordBox).toBeTruthy();
  expect(loginBox!.x).toBeGreaterThanOrEqual(urlBox!.x + urlBox!.width);
  expect(passwordBox!.x).toBeGreaterThanOrEqual(loginBox!.x + loginBox!.width);
  expect(Math.abs(urlBox!.y - loginBox!.y)).toBeLessThan(4);
  await copyLogin.click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe('fixture');
  await copyPassword.click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe('fixture-password');
  await expect(page.getByText('fixture-password')).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('NZBGet copy buttons appear after installation and use managed credentials', async ({
  page,
}) => {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  const options = {
    role: 'admin' as const,
    settingsSection: 'services',
    managedNzbget: true,
    stackProvisionState: 'queued',
  };
  const fixture = await installUiFixture(page, options);
  await page.goto('/');
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'NZBGet', exact: true })
    .click();
  const line = page.locator('.rail-identity:not(.inactive) .service-url-line');
  await expect(
    page
      .getByRole('complementary', { name: 'Service controls' })
      .getByRole('button'),
  ).toHaveCount(0);
  options.stackProvisionState = 'complete';
  await line.getByRole('button', { name: 'Copy NZBGet login' }).click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe('thelxinoe');
  await line.getByRole('button', { name: 'Copy NZBGet password' }).click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe('managed-password');
  expect(
    fixture.writes.some(
      (write) => write.path === '/admin/stack/managed-nzbget/login',
    ),
  ).toBe(true);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

for (const state of [
  'ready',
  'blocked',
  'recovery-required',
  'runtime-failure',
  'committed',
]) {
  test(`service remains running after ${state} update`, async ({ page }) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
      serviceUpdateState: state,
      stackCapabilities: {
        can_retire: true,
        can_remove: true,
        can_recreate: true,
      },
    });
    await page.goto('/');
    await selectRadarr(page);
    await expect(
      page.getByRole('button', { name: 'Restart', exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByRole('button', { name: 'Retire and keep data', exact: true }),
    ).toBeEnabled();
    await expect(
      page.getByText('Retiring removes the installation', { exact: false }),
    ).toBeVisible();
    if (state === 'ready')
      await expect(
        page.getByRole('button', {
          name: 'Install verified update',
          exact: true,
        }),
      ).toBeEnabled();
    if (state === 'recovery-required')
      await expect(
        page.getByRole('button', {
          name: 'Recover before activation',
          exact: true,
        }),
      ).toBeEnabled();
    const service = page.getByRole('article', { name: 'Radarr service' });
    await expect(
      service.locator('.rail-identity:not(.inactive) > .service-status'),
    ).toHaveText('Running');
    if (state === 'blocked' || state === 'committed')
      await expect(
        service.getByRole('region', { name: 'Updates' }),
      ).toContainText(
        state === 'blocked' ? 'Update incompatible' : 'Update complete',
      );
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

for (const scenario of [
  {
    endpoint: '/admin/managers',
    label: 'Acquisition managers',
    unaffectedService: 'Prowlarr',
    unaffectedVersion: '2.0.0',
  },
  {
    endpoint: '/admin/support',
    label: 'Support services',
    unaffectedService: 'Radarr',
    unaffectedVersion: '6.0.0',
  },
  {
    endpoint: '/admin/acquisition/users',
    label: 'Request approval settings',
    unaffectedService: 'Radarr',
    unaffectedVersion: '6.0.0',
  },
  {
    endpoint: '/admin/service-updates',
    label: 'Service updates',
    unaffectedService: 'Radarr',
    unaffectedVersion: '6.0.0',
  },
  {
    endpoint: '/admin/managers/containers',
    label: 'Docker container discovery',
    unaffectedService: 'Radarr',
    unaffectedVersion: '6.0.0',
  },
  {
    endpoint: '/admin/stack',
    label: 'Managed service runtime',
    unaffectedService: 'Radarr',
    unaffectedVersion: '6.0.0',
  },
]) {
  test(`media services isolate ${scenario.label.toLowerCase()} load failures`, async ({
    page,
  }) => {
    const failure = `Fixture ${scenario.label} failure`;
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
      endpointFailures: { [scenario.endpoint]: failure },
    });
    await page.goto('/');

    const banner = page.getByRole('alert', {
      name: `${scenario.label} load error`,
    });
    await expect(banner).toContainText(`Failed to load ${scenario.label}`);
    await expect(banner).toContainText(failure);
    await expect(banner).toHaveAttribute('data-tone', 'danger');
    await page
      .getByRole('navigation', { name: 'Select service' })
      .getByRole('button', { name: scenario.unaffectedService, exact: true })
      .click();

    await expect(
      page.getByRole('article', {
        name: `${scenario.unaffectedService} service`,
      }),
    ).toContainText(scenario.unaffectedVersion);
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

for (const state of ['connecting', 'blocked']) {
  test(`media service status prioritizes ${state} provisioning over runtime`, async ({
    page,
  }) => {
    const fixture = await installUiFixture(page, {
      role: 'admin',
      settingsSection: 'services',
      stackProvisionState: state,
    });
    await page.goto('/');
    await selectRadarr(page);
    const radarr = page.getByRole('article', { name: 'Radarr service' });
    await expect(
      radarr.locator('.rail-identity:not(.inactive) > .service-status'),
    ).toHaveText(state === 'connecting' ? 'Connecting API' : 'Setup blocked');
    if (state === 'connecting') {
      await expect(
        radarr
          .locator('.rail-identity:not(.inactive)')
          .getByRole('progressbar'),
      ).toBeVisible();
      await expect(
        radarr.locator('.rail-actions:not(.inactive)').getByRole('progressbar'),
      ).toHaveCount(0);
      const progress = radarr.locator(
        '.rail-identity:not(.inactive) .activity-bar > span',
      );
      expect(
        await progress.evaluate((node) => getComputedStyle(node).animationName),
      ).not.toBe('none');
      await page.emulateMedia({ reducedMotion: 'reduce' });
      expect(
        await progress.evaluate((node) => getComputedStyle(node).animationName),
      ).toBe('none');
    }
    expect(fixture.errors).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
  });
}

test('media services show request approval only for regular users', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
    approvalUsers: [{ id: 'member', username: 'Member', enabled: false }],
  });
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Automatic request approval' }),
  ).toBeVisible();
  await expect(page.getByRole('switch', { name: 'Member' })).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('mobile navigation opens over full-width content and restores the desktop sidebar', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  const sidebar = page.getByRole('complementary', {
    name: 'Application sidebar',
  });
  const menu = page.getByRole('button', { name: 'Open navigation' });
  const drawer = page.getByRole('dialog', { name: 'Navigation menu' });
  await expect(sidebar).toBeVisible();
  await expect(menu).toBeHidden();
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();

  for (const width of [320, 390, 720]) {
    await page.setViewportSize({ width, height: 844 });
    await expect(sidebar).toBeHidden();
    await expect(menu).toBeVisible();
    await expect(menu).toHaveAttribute('aria-expanded', 'false');
    await expect(page.locator('main')).toHaveCSS('width', `${width}px`);
    const contentBounds = await page.locator('main').boundingBox();
    await page.screenshot({ path: testInfo.outputPath(`mobile-${width}.png`) });

    await menu.click();
    await expect(drawer).toBeVisible();
    await expect(sidebar).toBeVisible();
    await expect(drawer.getByText('Thelxinoe', { exact: true })).toBeVisible();
    await expect(
      page.getByRole('button', { name: 'Expand sidebar' }),
    ).toBeHidden();
    expect(await page.locator('main').boundingBox()).toEqual(contentBounds);
    expect(await drawer.evaluate((node) => node.matches(':modal'))).toBe(true);
    await expect(
      drawer.getByRole('link', { name: 'Thelxinoe home' }),
    ).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(
      drawer.getByRole('button', { name: 'Close navigation' }),
    ).toBeFocused();
    await page.screenshot({
      path: testInfo.outputPath(`mobile-menu-${width}.png`),
    });

    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await expect(menu).toBeFocused();
    await menu.click();
    await drawer.getByRole('button', { name: 'Close navigation' }).click();
    await expect(drawer).toBeHidden();
    await expect(menu).toBeFocused();
    await menu.click();
    await page.mouse.click(width - 8, 100);
    await expect(drawer).toBeHidden();
    await menu.click();
    await drawer.getByRole('button', { name: 'Home', exact: true }).click();
    await expect(drawer).toBeHidden();
    await expect(
      page.getByRole('heading', { name: 'Discover', exact: true }),
    ).toBeVisible();
    await menu.click();
    await drawer.getByRole('button', { name: 'Settings', exact: true }).click();
    await expect(drawer).toBeHidden();
    await expect(menu).toBeFocused();
    await expect(
      page.getByRole('heading', { name: 'Settings', exact: true }),
    ).toBeVisible();
  }

  await menu.click();
  await page.setViewportSize({ width: 721, height: 844 });
  await expect(drawer).toBeHidden();
  await expect(menu).toBeHidden();
  await expect(sidebar).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Expand sidebar' }),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Expand sidebar' }).click();
  await expect(
    page.getByRole('button', { name: 'Collapse sidebar' }),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(drawer).toBeHidden();
  await expect(sidebar).toBeHidden();
  await page.reload();
  await expect(sidebar).toBeHidden();
  await expect(menu).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(sidebar).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Collapse sidebar' }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath('desktop-navigation.png'),
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('mobile settings show every category in a wrapping grid', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { role: 'admin' });
  for (const width of [720, 480, 320]) {
    await page.setViewportSize({ width, height: 850 });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Settings navigation' });
    expect(
      await nav.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
    const bounds = await nav.boundingBox();
    const workspace = await page.locator('.workspace-scroll').boundingBox();
    expect(bounds!.width).toBeLessThanOrEqual(workspace!.width);
    expect(bounds!.height).toBeLessThan(520);
    await expect(page.locator('.settings-panels > .panel').first()).toHaveCSS(
      'padding',
      '20px 0px 0px',
    );
    await nav
      .getByRole('button', { name: 'Audit', exact: true })
      .scrollIntoViewIfNeeded();
    await expect(
      nav.getByRole('button', { name: 'Audit', exact: true }),
    ).toBeInViewport();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  }
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('display, playback and skipping preferences save automatically', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  await page.goto('/');
  await page
    .getByRole('combobox', { name: 'Display timezone', exact: true })
    .selectOption('Europe/Paris');
  await expect
    .poll(() => fixture.writes)
    .toContainEqual({
      path: '/me/preferences',
      method: 'PUT',
      body: { timezone: 'Europe/Paris', time_format: null },
    });
  await page
    .getByRole('combobox', { name: 'Display time format', exact: true })
    .selectOption('12h');
  await expect
    .poll(() => fixture.writes)
    .toContainEqual({
      path: '/me/preferences',
      method: 'PUT',
      body: { timezone: 'Europe/Paris', time_format: '12h' },
    });
  await page.getByRole('button', { name: 'Smaller media cards' }).click();
  await expect(page.getByText('7 columns', { exact: true })).toBeVisible();
  await page.getByText('Fade watched videos', { exact: true }).click();
  await expect(
    page.getByRole('switch', { name: 'Fade watched videos' }),
  ).not.toBeChecked();
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/me/appearance' &&
          JSON.stringify(write.body) === '{"fade_watched":false}',
      ),
    )
    .toBe(true);
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Playback', exact: true })
    .click();
  const choices = page.getByRole('group', { name: 'Intro skipping' });
  await choices.getByRole('button', { name: 'Ignore', exact: true }).click();
  await expect(
    choices.getByRole('button', { name: 'Ignore', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await expect
    .poll(() => fixture.writes.filter((write) => write.path === '/me/segments'))
    .toHaveLength(1);
  expect(
    fixture.writes.find((write) => write.path === '/me/segments')?.body,
  ).toEqual({ Intro: 'Ignore', Recap: 'Ask', Credits: 'Ask', Preview: 'Ask' });
  await page.getByLabel('Default quality').selectOption('original');
  await expect
    .poll(() =>
      fixture.writes.filter((write) => write.path === '/playback/preferences'),
    )
    .toHaveLength(1);
  await page.getByLabel('Audio language').fill('fra');
  await page.getByLabel('Audio language').press('Tab');
  await expect
    .poll(() =>
      fixture.writes.filter((write) => write.path === '/playback/preferences'),
    )
    .toHaveLength(2);
  expect(
    fixture.writes
      .filter((write) => write.path === '/playback/preferences')
      .at(-1)?.body,
  ).toMatchObject({ quality: 'original', audio_language: 'fra' });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('failed automatic saves revert edits and allow another selection', async ({
  page,
}) => {
  const fixture = await installUiFixture(page);
  let fail = true;
  await page.route('**/api/v1/me/preferences', async (route) => {
    if (route.request().method() !== 'PUT' || !fail) return route.fallback();
    fail = false;
    await route.fulfill({
      status: 503,
      json: {
        error: { code: 'unavailable', message: 'Temporary connection failure' },
      },
    });
  });
  await page.goto('/');
  await page
    .getByRole('combobox', { name: 'Display timezone', exact: true })
    .selectOption('Europe/Paris');
  await expect(page.getByRole('alert')).toContainText(
    'Temporary connection failure',
  );
  await expect(
    page.getByRole('combobox', { name: 'Display timezone', exact: true }),
  ).toHaveValue('');
  await page
    .getByRole('combobox', { name: 'Display timezone', exact: true })
    .selectOption('Europe/Paris');
  await expect
    .poll(() => fixture.writes)
    .toContainEqual({
      path: '/me/preferences',
      method: 'PUT',
      body: { timezone: 'Europe/Paris', time_format: null },
    });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('library cards retain their aspect ratio, hover actions and Ctrl-wheel zoom', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { section: 'Movies' });
  await page.goto('/');
  const card = page.locator('.library-tile').first();
  await expect(card).toBeVisible();
  await expect(card.locator('.tile-art')).toHaveCSS('aspect-ratio', '2 / 3');
  await card.hover();
  await expect(card.locator('.card-actions')).toHaveCSS('opacity', '1');
  const width = (await card.boundingBox())!.width;
  await page.keyboard.down('Control');
  await page.mouse.wheel(0, -100);
  await page.keyboard.up('Control');
  await expect
    .poll(async () => (await card.boundingBox())!.width)
    .toBeGreaterThan(width);
  await expect
    .poll(() =>
      fixture.writes.some(
        (write) =>
          write.path === '/me/appearance' &&
          JSON.stringify(write.body) === '{"card_columns":5}',
      ),
    )
    .toBe(true);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('the sign-in button still submits the form by keyboard', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('fixture');
  await page.getByLabel('Password', { exact: true }).fill('fixture password');
  const button = page.getByRole('button', { name: 'Sign in', exact: true });
  expect(
    await button.evaluate(
      (element) =>
        element.getBoundingClientRect().width ===
        element.closest('form')!.getBoundingClientRect().width,
    ),
  ).toBe(true);
  await page.getByLabel('Password', { exact: true }).press('Enter');
  await expect(
    page.getByRole('navigation', { name: 'Settings navigation' }),
  ).toBeVisible();
  expect(fixture.writes).toContainEqual({
    path: '/auth/login',
    method: 'POST',
    body: { username: 'fixture', password: 'fixture password' },
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('skipping stays responsive during saves, animates, and reverts a failed latest edit', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { settingsSection: 'playback' });
  const writes: Record<string, string>[] = [];
  const complete: (() => void)[] = [];
  let fail = false;
  await page.route('**/api/v1/me/segments', async (route) => {
    if (route.request().method() !== 'PUT') return route.fallback();
    writes.push(route.request().postDataJSON());
    await new Promise<void>((resolve) => complete.push(resolve));
    await route.fulfill(
      fail
        ? {
            status: 503,
            json: { error: { code: 'unavailable', message: 'Save failed' } },
          }
        : { json: {} },
    );
  });
  await page.goto('/');
  const intro = page.getByRole('group', { name: 'Intro skipping' });
  const recap = page.getByRole('group', { name: 'Recap skipping' });
  const ignore = intro.getByRole('button', { name: 'Ignore', exact: true });
  const auto = intro.getByRole('button', { name: 'Auto', exact: true });
  await expect(auto).toBeEnabled();
  const selection = intro.locator('[data-choice-selection]');
  // Freeze the animation at an intermediate frame to verify actual movement.
  await page.evaluate(() => {
    const animate = Element.prototype.animate;
    Element.prototype.animate = function (keyframes, options) {
      const animation = animate.call(this, keyframes, options);
      if (this.hasAttribute('data-choice-selection')) animation.pause();
      return animation;
    };
  });
  await intro.evaluate((element) => {
    const button = element.querySelector<HTMLButtonElement>(
      'button[aria-label="Auto"]',
    )!;
    button.click();
  });
  await expect.poll(() => complete.length).toBe(1);
  await expect(auto).toHaveAttribute('aria-pressed', 'true');
  await expect(ignore).toBeEnabled();
  const motion = await selection.evaluate((element) => {
    const animation = element
      .getAnimations()
      .find(
        (item) =>
          item.effect instanceof KeyframeEffect &&
          item.effect
            .getKeyframes()
            .some((frame) => frame.translate !== undefined),
      );
    if (!animation) return null;
    animation.pause();
    animation.currentTime = 80;
    const rect = element.getBoundingClientRect();
    const parent = element.parentElement!;
    const from = parent.querySelector('button')!.getBoundingClientRect();
    const to = parent
      .querySelector('[aria-pressed="true"]')!
      .getBoundingClientRect();
    return { left: rect.left, from: from.left, to: to.left };
  });
  expect(motion).not.toBeNull();
  expect(motion!.left).toBeGreaterThan(motion!.from);
  expect(motion!.left).toBeLessThan(motion!.to);
  await selection.evaluate((element) =>
    element.getAnimations().forEach((animation) => animation.finish()),
  );
  await ignore.click();
  await recap.getByRole('button', { name: 'Ignore', exact: true }).click();
  await expect(ignore).toHaveAttribute('aria-pressed', 'true');
  expect(writes).toHaveLength(1);
  fail = true;
  complete[0]();
  await expect.poll(() => complete.length).toBe(2);
  // Failure of the older request must not roll back the newer choices.
  await expect(ignore).toHaveAttribute('aria-pressed', 'true');
  expect(writes[1]).toMatchObject({ Intro: 'Ignore', Recap: 'Ignore' });
  fail = false;
  complete[1]();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await auto.click();
  await expect.poll(() => complete.length).toBe(3);
  fail = true;
  complete[2]();
  await expect(page.getByRole('alert')).toContainText('were reverted');
  await expect(ignore).toHaveAttribute('aria-pressed', 'true');
  await expect(
    recap.getByRole('button', { name: 'Ignore', exact: true }),
  ).toHaveAttribute('aria-pressed', 'true');
  await expect(auto).toBeEnabled();
  await expect(page.getByText('Saved', { exact: true })).toHaveCount(0);
  await page.screenshot({
    path: 'test-results/settings-optimistic.png',
    fullPage: true,
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('update choices save eagerly, roll back failures, and respect reduced motion', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  let finish: (() => void) | undefined;
  await page.route('**/api/v1/admin/product-update/policy', async (route) => {
    await new Promise<void>((resolve) => (finish = resolve));
    await route.fulfill({
      status: 503,
      json: { error: { code: 'unavailable', message: 'Policy save failed' } },
    });
  });
  await page.goto('/');
  const choices = page.getByRole('group', { name: 'Update policy' });
  const automatic = choices.getByRole('button', {
    name: 'Automatic',
    exact: true,
  });
  const notify = choices.getByRole('button', { name: 'Notify', exact: true });
  await automatic.click();
  await expect(automatic).toHaveAttribute('aria-pressed', 'true');
  await expect(notify).toBeEnabled();
  await expect(choices.locator('[data-choice-selection]')).toHaveCSS(
    'transition-property',
    'none',
  );
  await expect.poll(() => Boolean(finish)).toBe(true);
  finish!();
  await expect(page.getByRole('alert')).toContainText('Policy save failed');
  await expect(notify).toHaveAttribute('aria-pressed', 'true');
  await expect(
    choices.getByRole('button', { name: 'Manual', exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', { name: 'Check signed releases' }),
  ).toHaveCount(0);
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await expect(nav.getByRole('heading')).toHaveCount(0);
  await expect(nav.getByText('Administration', { exact: true })).toBeVisible();
  await page.screenshot({
    path: 'test-results/settings-update-policy.png',
    fullPage: true,
  });
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('matching service images are up to date with no update action', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'services',
  });
  const currentImage = 'ghcr.io/example/radarr:stable';
  await page.route('**/api/v1/admin/service-updates', async (route) => {
    await route.fulfill({
      json: {
        policies: [
          {
            service_id: 'managed-radarr',
            policy: 'notify',
            window_start: 3,
            window_end: 5,
            candidate: currentImage,
            checked_at: 1700000000,
            error: null,
          },
        ],
        items: [],
        services: [{ id: 'managed-radarr', kind: 'radarr' }],
        timezone: 'UTC',
        server_policy: { policy: 'notify', window_start: 3, window_end: 5 },
      },
    });
  });
  await page.goto('/');
  await selectRadarr(page);
  const updates = page.getByRole('region', { name: 'Updates', exact: true });
  await expect(updates.getByText('Up to date.', { exact: true })).toBeVisible();
  await expect(
    updates.getByRole('button', { name: 'Check compatibility' }),
  ).toHaveCount(0);
  await expect(
    page.getByRole('button', {
      name: /Check stable updates|Discover stable releases/,
    }),
  ).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('failed playback saves preserve newer text still being edited', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { settingsSection: 'playback' });
  let finish: (() => void) | undefined;
  await page.route('**/api/v1/playback/preferences', async (route) => {
    if (route.request().method() !== 'PUT') return route.fallback();
    await new Promise<void>((resolve) => (finish = resolve));
    await route.fulfill({
      status: 503,
      json: { error: { code: 'unavailable', message: 'Playback save failed' } },
    });
  });
  await page.goto('/');
  await page.getByLabel('Default quality').selectOption('original');
  await page.getByLabel('Audio language').fill('fra');
  await expect.poll(() => Boolean(finish)).toBe(true);
  finish!();
  await expect(page.getByRole('alert')).toContainText('Playback save failed');
  await expect(page.getByLabel('Default quality')).toHaveValue('auto');
  await expect(page.getByLabel('Audio language')).toHaveValue('fra');
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
