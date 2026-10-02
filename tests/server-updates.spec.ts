import { expect, test, type Page } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';
import type { ServerUpdateStatus } from '../frontend/src/lib/server-updates';

async function setup(
  page: Page,
  options: { desktop?: boolean; loseInstallResponse?: boolean } = {},
) {
  if (options.desktop)
    await page.addInitScript(() => {
      const bridge = window as unknown as {
        isTauri: boolean;
        __TAURI_INTERNALS__: unknown;
        __TAURI_EVENT_PLUGIN_INTERNALS__: unknown;
      };
      bridge.isTauri = true;
      bridge.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      let callback = 0;
      bridge.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: 'main' },
          currentWebview: { label: 'main' },
        },
        transformCallback: () => ++callback,
        unregisterCallback() {},
        async invoke(command: string, args: Record<string, unknown> = {}) {
          if (command === 'server_url') return location.origin;
          if (command === 'backend_request') {
            const response = await fetch(`/api/v1${args.path}`, {
              method: String(args.method),
              headers: { 'Content-Type': 'application/json' },
              ...(args.body ? { body: JSON.stringify(args.body) } : {}),
            });
            return { status: response.status, body: await response.json() };
          }
          if (command === 'mpv_state')
            return { status: 'stopped', media_id: '' };
          if (command === 'tools_get') return { tools: [], mpv: {} };
          if (command === 'plugin:app|version') return '0.1.0';
          if (command === 'desktop_update_status')
            return {
              installed: '0.1.0',
              phase: 'idle',
              policy: 'notify',
              release: null,
              error: null,
              received: 0,
              total: 0,
              checked_at: null,
            };
          if (command === 'plugin:event|listen') return ++callback;
          return false;
        },
      };
    });
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  const status: ServerUpdateStatus = {
    version: '0.1.0',
    timezone: 'UTC',
    configured: true,
    policy: { policy: 'notify', window_start: 3, window_end: 5 },
    release: { version: '0.1.1', notes: 'Local candidate' },
    observation: { checked_at: 1, error: null },
    controller: { items: [] },
  };
  let offline = false;
  const commands: { action: string; body: unknown }[] = [];
  await page.route('**/api/v1/admin/product-update{,/**}', async (route) => {
    if (offline) return route.abort('connectionrefused');
    const action = new URL(route.request().url()).pathname.split('/').at(-1)!;
    if (route.request().method() === 'POST') {
      commands.push({ action, body: route.request().postDataJSON() });
      if (action === 'install')
        status.request = {
          id: 'update',
          version: '0.1.1',
          previous_version: '0.1.0',
          state: 'pending',
          error: null,
        };
      if (action === 'install' && options.loseInstallResponse)
        return route.abort('connectionreset');
      return route.fulfill({ json: status.request ?? {} });
    }
    return route.fulfill({ json: status });
  });
  if (options.desktop)
    await page.addInitScript(() => {
      localStorage.setItem(
        `thelxinoe:${location.origin}:layout-fixture:navigation`,
        JSON.stringify({ section: 'Settings', settingsSection: 'server' }),
      );
    });
  await page.goto('/');
  const version = page.getByLabel('Product version', { exact: true });
  await expect(
    version.getByRole('button', {
      name: 'Update server to 0.1.1',
      exact: true,
    }),
  ).toBeVisible();
  const refresh = () =>
    page.evaluate(() =>
      window.dispatchEvent(new Event('thelxinoe-product-update')),
    );
  function stage(stage: string) {
    status.controller.items = [
      {
        id: 'update',
        version: '0.1.1',
        previous_version: '0.1.0',
        stage,
        error: null,
        snapshot_ready: true,
        recovery_tested: true,
        activation_crossed: false,
      },
    ];
  }
  return {
    fixture,
    status,
    version,
    commands,
    refresh,
    stage,
    offline: (value: boolean) => {
      offline = value;
    },
  };
}

test('desktop reconciles a lost install acceptance response through pending and committed status', async ({
  page,
}, testInfo) => {
  const flow = await setup(page, { desktop: true, loseInstallResponse: true });
  let reloads = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) reloads++;
  });
  await flow.version
    .getByRole('button', { name: 'Update server to 0.1.1', exact: true })
    .click();
  await expect(flow.version.getByRole('button').first()).toHaveAccessibleName(
    'Waiting for the server to be idle',
  );
  flow.stage('preparing');
  await flow.refresh();
  await expect(flow.version.getByRole('button').first()).toHaveAccessibleName(
    'Downloading server update',
  );
  flow.status.version = '0.1.1';
  flow.status.release = null;
  flow.status.request!.state = 'completed';
  flow.stage('committed');
  await flow.refresh();
  await expect(flow.version.getByRole('status')).toHaveText('Updated to 0.1.1');
  await expect(flow.version.locator('strong')).toHaveText('0.1.1');
  await expect(flow.version).not.toContainText('Update could not complete');
  expect(reloads).toBe(0);
  expect(flow.fixture.errors).toEqual([]);
  await testInfo.attach('desktop-response-loss-committed', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('Orbit carries the full flow in one fixed, keyboard-accessible control', async ({
  page,
}, testInfo) => {
  const flow = await setup(page);
  const { version } = flow;
  const button = version.getByRole('button');
  await button.focus();
  await expect(version.getByRole('tooltip')).toHaveCSS('opacity', '1');
  await page.keyboard.press('Escape');
  await expect(version.getByRole('tooltip')).toBeHidden();
  const before = await version.locator('strong').boundingBox();
  await testInfo.attach('available', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.keyboard.press('Enter');
  await expect(button).toHaveAccessibleName(
    'Waiting for the server to be idle',
  );
  for (const [stage, title] of [
    ['preparing', 'Downloading server update'],
    ['snapshotting', 'Saving a recovery copy'],
    ['validating', 'Checking the update'],
    ['ready', 'Waiting for the server to be idle'],
    ['preparing-activation', 'Installing 0.1.1'],
  ]) {
    flow.stage(stage);
    await flow.refresh();
    await expect(button).toHaveAccessibleName(title);
    await expect(button).toHaveAttribute('aria-disabled', 'true');
    await button.dispatchEvent('click');
    await expect(version.locator('strong')).toHaveText('0.1.0');
    expect(await version.locator('strong').boundingBox()).toEqual(before);
  }
  // A reachable candidate is not accepted until controller health checks finish.
  flow.status.version = '0.1.1';
  flow.status.controller = { items: [], error: 'Controller unavailable' };
  await flow.refresh();
  await expect(button).toHaveAccessibleName('Reconnecting to the server');
  await expect(version.locator('strong')).toHaveText('0.1.0');
  delete flow.status.controller.error;
  flow.stage('activating');
  await flow.refresh();
  await expect(version.locator('strong')).toHaveText('0.1.0');
  flow.status.release = null;
  flow.stage('committed');
  await flow.refresh();
  await expect(version.locator('strong')).toHaveText('0.1.1');
  await expect(version.getByRole('status')).toHaveText('Updated to 0.1.1');
  expect(flow.commands).toEqual([
    { action: 'install', body: { version: '0.1.1' } },
  ]);
  await expect(
    page.getByRole('heading', { name: 'Server updates' }),
  ).toBeVisible();
  expect(flow.fixture.errors).toEqual([]);
  await testInfo.attach('accepted', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('server settings keep their layout through update states and maintenance reconnects', async ({
  page,
}, testInfo) => {
  await page.clock.install();
  const flow = await setup(page);
  await expect(
    page.getByText('No active playback.', { exact: true }),
  ).toBeVisible();
  const bounds = () =>
    page.evaluate(() =>
      Object.fromEntries(
        [
          'Product version',
          'Server update settings',
          'Server display defaults',
          'Administration overview',
        ].map((label) => {
          const box = document
            .querySelector(`[aria-label="${label}"]`)!
            .getBoundingClientRect();
          return [
            label,
            { x: box.x, y: box.y, width: box.width, height: box.height },
          ];
        }),
      ),
    );
  const before = await bounds();
  const frames = [{ stage: 'available', bounds: before }];
  await flow.version.getByRole('button').click();
  for (const stage of [
    'preparing',
    'snapshotting',
    'validating',
    'ready',
    'activating',
  ]) {
    flow.stage(stage);
    await flow.refresh();
    await expect(flow.version.getByRole('button')).toHaveAttribute(
      'aria-disabled',
      'true',
    );
    frames.push({ stage, bounds: await bounds() });
    expect(await bounds()).toEqual(before);
  }
  let sendEvent: ((kind: string) => void) | undefined;
  let disconnect: (() => void) | undefined;
  let connections = 0;
  let eventId = 0;
  await page.routeWebSocket(/\/api\/v1\/events(?:\?|$)/, (socket) => {
    connections++;
    disconnect = () => socket.close();
    sendEvent = (kind) =>
      socket.send(JSON.stringify({ id: ++eventId, kind, payload: {} }));
  });
  await page.route('**/api/v1/auth/event-ticket', (route) =>
    route.fulfill({
      json: { ticket: 'layout', cursor: 0, epoch: 'layout', version: '0.1.0' },
    }),
  );
  await page.reload();
  await expect.poll(() => Boolean(sendEvent)).toBe(true);
  await expect(
    page.getByText('No active playback.', { exact: true }),
  ).toBeVisible();
  await page.route(
    /\/api\/v1\/(auth\/sessions|me\/appearance|admin\/operations)$/,
    (route) =>
      route.fulfill({
        status: 503,
        json: {
          error: {
            code: 'maintenance',
            message: 'The server is preparing an update. Reconnect shortly.',
          },
        },
      }),
  );
  sendEvent!('jobs.changed');
  await page.clock.fastForward(31000);
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(await bounds()).toEqual(before);
  frames.push({ stage: 'maintenance', bounds: await bounds() });
  disconnect!();
  await page.clock.fastForward(1000);
  await expect.poll(() => connections).toBe(2);
  await expect(
    page.getByText('Appearance preferences could not be loaded.'),
  ).toHaveCount(0);
  expect(await bounds()).toEqual(before);
  await page.route(/\/api\/v1\/(auth\/sessions|admin\/operations)$/, (route) =>
    route.abort('connectionrefused'),
  );
  sendEvent!('jobs.changed');
  await page.clock.fastForward(31000);
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(await bounds()).toEqual(before);
  frames.push({ stage: 'reconnecting', bounds: await bounds() });
  await testInfo.attach('settings-positions', {
    body: JSON.stringify(frames, null, 2),
    contentType: 'application/json',
  });
  await testInfo.attach('maintenance', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('Orbit survives lost connections, checks without reinstalling, and retries recovered releases', async ({
  page,
}, testInfo) => {
  await page.clock.install();
  const flow = await setup(page);
  const button = flow.version.getByRole('button');
  await button.click();
  flow.stage('validating');
  await flow.refresh();
  await expect(button).toHaveAccessibleName('Checking the update');
  flow.offline(true);
  await flow.refresh();
  await expect(button).toHaveAccessibleName('Reconnecting to the server');
  await page.clock.fastForward(115000);
  await expect(button).toHaveAccessibleName('Reconnecting to the server');
  await page.clock.fastForward(10000);
  await expect(button).toHaveAccessibleName('Check server connection');
  await button.click();
  expect(flow.commands).toHaveLength(1);
  flow.offline(false);
  flow.stage('recovering');
  await flow.refresh();
  await expect(button).toHaveAccessibleName('Restoring the previous version');
  flow.stage('recovered');
  await flow.refresh();
  await expect(button).toHaveAccessibleName('Retry server update');
  await expect(flow.version.locator('strong')).toHaveText('0.1.0');
  await testInfo.attach('recovered', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await button.click();
  expect(flow.commands.map((c) => c.action)).toEqual(['install', 'install']);
  expect(flow.fixture.errors).toEqual([]);
});

test('download tracks bytes without pinning its tooltip, and controller reconnect gets two minutes', async ({
  page,
}, testInfo) => {
  await page.clock.install();
  const flow = await setup(page);
  const button = flow.version.getByRole('button').first();
  await button.hover();
  await expect(flow.version.getByRole('tooltip')).toBeVisible();
  await button.click();
  flow.stage('preparing');
  for (const received of [25, 65, 100]) {
    flow.status.controller.items[0].download = {
      component: 'server',
      received,
      total: 100,
    };
    await flow.refresh();
    await expect(flow.version.getByRole('progressbar')).toHaveAttribute(
      'aria-valuenow',
      String(received),
    );
    await expect(flow.version.locator('circle').last()).toHaveAttribute(
      'stroke-dasharray',
      `${received} 100`,
    );
    await expect(flow.version.getByRole('tooltip')).toHaveText(
      'Downloading server update',
    );
  }
  await page.mouse.move(0, 0);
  await expect(flow.version.getByRole('tooltip')).toBeHidden();
  let releaseObservation!: () => void;
  const observation = new Promise<void>((resolve) => {
    releaseObservation = resolve;
  });
  let observations = 0;
  await page.route('**/api/v1/admin/product-update', async (route) => {
    observations++;
    await observation;
    await route.fallback();
  });
  flow.status.controller.error = 'Controller unavailable';
  try {
    await flow.refresh();
    await expect.poll(() => observations).toBeGreaterThan(0);
    await expect(button).toHaveAccessibleName('Downloading server update');
  } finally {
    releaseObservation();
  }
  await expect(button).toHaveAccessibleName('Reconnecting to the server');
  const deadlineStates = [
    { stage: 'observed', time: await page.evaluate(() => Date.now()) },
  ];
  await page.clock.fastForward(115000);
  await expect(button).toHaveAccessibleName('Reconnecting to the server');
  deadlineStates.push({
    stage: 'before-deadline',
    time: await page.evaluate(() => Date.now()),
  });
  await testInfo.attach('reconnecting', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.clock.fastForward(10000);
  await expect(button).toHaveAccessibleName('Check server connection');
  deadlineStates.push({
    stage: 'after-deadline',
    time: await page.evaluate(() => Date.now()),
  });
  await testInfo.attach('controller-reconnect-deadline', {
    body: JSON.stringify(deadlineStates, null, 2),
    contentType: 'application/json',
  });
  delete flow.status.controller.error;
  flow.stage('committed');
  flow.status.version = '0.1.1';
  flow.status.release = null;
  await flow.refresh();
  await expect(flow.version.getByRole('status')).toHaveText('Updated to 0.1.1');
  await flow.version
    .getByRole('button', { name: 'Rollback', exact: true })
    .click();
  const modal = page.getByRole('dialog');
  await expect(modal).toContainText('Rollback to 0.1.0');
  await expect(
    modal.getByRole('button', { name: 'Restore 0.1.0', exact: true }),
  ).toBeDisabled();
  await modal.getByLabel('Type RESTORE to continue').fill('RESTORE');
  await testInfo.attach('rollback', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  let recoveryRequests = 0;
  await page.route(
    '**/api/v1/admin/product-update/*/recover',
    async (route) => {
      if (++recoveryRequests === 1)
        await route.fulfill({
          status: 409,
          json: {
            error: {
              code: 'conflict',
              message: 'Wait for current requests to finish and try again',
            },
          },
        });
      else await route.fallback();
    },
  );
  const restoreButton = modal.getByRole('button', {
    name: 'Restore 0.1.0',
    exact: true,
  });
  await restoreButton.click();
  await expect.poll(() => recoveryRequests).toBe(1);
  await expect(restoreButton).toBeDisabled();
  await expect(restoreButton).toHaveAttribute('aria-busy', 'true');
  await testInfo.attach('rollback-waiting-for-idle', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await page.clock.runFor(1500);
  await expect(modal).toHaveCount(0);
  expect(recoveryRequests).toBe(2);
  expect(flow.commands.at(-1)).toEqual({
    action: 'recover',
    body: { confirm: true },
  });
  await expect(page.getByLabel('Client update available')).toHaveCount(0);
  expect(flow.fixture.errors).toEqual([]);
});

test('web clients reload accepted updates and rollbacks without a popup', async ({
  page,
}, testInfo) => {
  await page.clock.install();
  const flow = await setup(page);
  let reloads = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) reloads++;
  });
  flow.status.version = '0.1.1';
  flow.status.release = null;
  flow.stage('activating');
  await page.evaluate(() =>
    window.dispatchEvent(
      new CustomEvent('thelxinoe-web-update', { detail: '0.1.1' }),
    ),
  );
  await page.clock.fastForward(6000);
  expect(reloads).toBe(0);
  await expect(page.getByLabel('Client update available')).toHaveCount(0);
  flow.stage('committed');
  await page.clock.fastForward(3000);
  await expect.poll(() => reloads).toBe(1);
  await expect(flow.version.locator('strong')).toHaveText('0.1.1');
  flow.status.version = '0.1.0';
  flow.stage('restored');
  await page.evaluate(() =>
    window.dispatchEvent(
      new CustomEvent('thelxinoe-web-update', { detail: '0.1.0' }),
    ),
  );
  await expect.poll(() => reloads).toBe(2);
  await expect(flow.version.locator('strong')).toHaveText('0.1.0');
  await testInfo.attach('restored-web', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('Orbit remains usable at narrow widths, with reduced motion and both themes', async ({
  page,
}, testInfo) => {
  const flow = await setup(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const theme of ['light', 'dark']) {
    await page.evaluate(
      (theme) => (document.documentElement.dataset.theme = theme),
      theme,
    );
    for (const width of [390, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await flow.version.scrollIntoViewIfNeeded();
      await flow.version.getByRole('button').focus();
      await expect(flow.version.getByRole('tooltip')).toHaveCSS('opacity', '1');
      const bounds = await flow.version.getByRole('tooltip').boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(0);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      await expect
        .poll(() =>
          flow.version.evaluate(
            (node) =>
              node
                .getAnimations({ subtree: true })
                .filter((a) => a.playState === 'running').length,
          ),
        )
        .toBe(0);
      await testInfo.attach(`${theme}-${width}`, {
        body: await page.screenshot({
          path: testInfo.outputPath(`${theme}-${width}.png`),
        }),
        contentType: 'image/png',
      });
    }
  }
  expect(flow.fixture.errors).toEqual([]);
});
