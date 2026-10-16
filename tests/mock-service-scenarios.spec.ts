import { expect, test, type Page, type TestInfo } from '@playwright/test';

test.beforeEach(async ({ request }) => {
  await request.post('/__mock/reset');
});

async function controls(page: Page) {
  const panel = page.getByRole('region', { name: 'Service scenarios' });
  if (!(await panel.isVisible()))
    await page
      .getByRole('button', { name: 'Service scenarios', exact: true })
      .click();
  return panel;
}

async function scenario(
  page: Page,
  preset: string,
  kind = 'prowlarr',
  paused = false,
) {
  const panel = await controls(page);
  await panel.getByLabel('Preview service', { exact: true }).selectOption(kind);
  await panel.getByLabel('Scenario', { exact: true }).selectOption(preset);
  const reloaded = page.waitForEvent('load');
  await panel
    .getByRole('button', { name: 'Load scenario', exact: true })
    .click();
  await reloaded;
  await expect(page).toHaveURL(new RegExp(`#admin/services/${kind}$`));
  await expect(
    page.getByRole('heading', {
      name:
        kind === 'nzbget' ? 'NZBGet' : kind[0].toUpperCase() + kind.slice(1),
      exact: true,
    }),
  ).toBeVisible();
  const loaded = await controls(page);
  if (paused) {
    await loaded
      .getByRole('checkbox', { name: 'Pause transfer progress' })
      .check();
    await loaded.getByRole('button', { name: 'Apply controls' }).click();
  }
  return loaded;
}

async function evidence(page: Page, info: TestInfo, name: string) {
  await info.attach(name, {
    body: await page.screenshot({ fullPage: true }),
    contentType: 'image/png',
  });
  await info.attach(`${name}-state`, {
    body: JSON.stringify(
      await (await page.request.get('/__mock/services')).json(),
      null,
      2,
    ),
    contentType: 'application/json',
  });
}

test.afterEach(async ({ page, request }) => {
  expect(
    (await (await request.get('/__mock/diagnostics')).json()).unhandled,
  ).toEqual([]);
  await expect(page.locator('[data-mock-controls] [role="alert"]')).toHaveCount(
    0,
  );
});

test('server actions stay pending for three seconds while background reads stay responsive', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  const panel = await scenario(page, 'owned');
  await expect(panel.getByLabel('Response delay', { exact: true })).toHaveValue(
    '3000',
  );
  await panel.getByRole('button', { name: 'Close service scenarios' }).click();
  const restart = page.getByRole('button', { name: 'Restart', exact: true });
  const finished = page.waitForResponse(
    (response) =>
      response.url().endsWith('/admin/stack/managed-prowlarr/action') &&
      response.request().method() === 'POST',
  );
  const started = Date.now();
  await restart.click();
  await expect(restart).toBeDisabled();
  const runtime = await page.request.get('/api/v1/admin/stack');
  expect(runtime.ok()).toBe(true);
  await expect(restart).toBeDisabled();
  await evidence(page, info, 'restart-in-progress');
  await finished;
  const elapsed = Date.now() - started;
  expect(elapsed).toBeGreaterThanOrEqual(2900);
  await expect(restart).toBeEnabled();
  await info.attach('server-action-timing', {
    body: JSON.stringify({ action: 'restart', elapsed_ms: elapsed }),
    contentType: 'application/json',
  });
});

test('service scenarios switch between unconfigured, Connected and Owned and reset', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  await scenario(page, 'unconfigured');
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toHaveCount(0);
  await scenario(page, 'connected');
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toBeVisible();
  await scenario(page, 'owned');
  await expect(
    page.getByRole('region', { name: 'Connected service' }),
  ).toHaveCount(0);
  await evidence(page, info, 'owned');
  await (
    await controls(page)
  )
    .getByRole('button', { name: 'Reset all services', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toHaveCount(0);
});

test('blocked prerequisites show a blocked step and prevent transfer approval', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  await scenario(page, 'blocked');
  await page
    .getByRole('button', { name: 'Continue ownership review', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  const prerequisite = dialog
    .getByRole('navigation', { name: 'Ownership transfer steps' })
    .getByRole('button', { name: /^1\. Check prerequisites:/ });
  await expect(prerequisite).toHaveAttribute('data-status', 'blocked');
  await expect(prerequisite).toHaveAttribute('aria-current', 'step');
  await expect(dialog).toContainText(
    'The original service has active downloads.',
  );
  await expect(
    dialog.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toHaveCount(0);
  await evidence(page, info, 'blocked-prerequisites');
});

test('connection failures and response delay reach the real connection controls', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  let panel = await scenario(page, 'connected');
  await panel
    .getByLabel('Connection result', { exact: true })
    .selectOption('credentials');
  await panel
    .getByLabel('Response delay', { exact: true })
    .selectOption('1500');
  await panel
    .getByRole('button', { name: 'Apply controls', exact: true })
    .click();
  const check = page.getByRole('button', {
    name: 'Test connection',
    exact: true,
  });
  await check.click();
  await expect(check).toBeDisabled();
  await expect(
    page
      .getByText('Invalid API key or login credentials.', { exact: false })
      .first(),
  ).toBeVisible();
  await evidence(page, info, 'connection-rejected');
  panel = await controls(page);
  await panel
    .getByLabel('Connection result', { exact: true })
    .selectOption('success');
  await panel.getByLabel('Response delay', { exact: true }).selectOption('0');
  await panel
    .getByRole('button', { name: 'Apply controls', exact: true })
    .click();
  await check.click();
  await expect(
    page.getByRole('region', { name: 'Connected service' }),
  ).toContainText('Verified');
});

test('manual transfer steps survive dialog dismissal and release ownership only after checks', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  await scenario(page, 'review', 'prowlarr', true);
  await page
    .getByRole('button', { name: 'Continue ownership review', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  await expect(
    dialog.getByRole('region', { name: 'Deployment changes' }),
  ).toBeVisible();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await dialog
    .getByRole('button', { name: "Retire the whole project's automation" })
    .click();
  await expect(
    dialog.getByRole('switch', {
      name: 'No automation or manual command will run against the project',
    }),
  ).toBeVisible();
  for (const toggle of await dialog.getByRole('switch').all())
    await toggle.check();
  await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
  await dialog
    .getByRole('switch', {
      name: 'I reviewed every change, and I understand the original stops',
    })
    .check();
  await dialog
    .getByRole('button', { name: 'Take ownership', exact: true })
    .click();
  await expect(
    dialog
      .getByRole('navigation', { name: 'Ownership transfer steps' })
      .getByRole('button', { name: /^5\. Stop the original: In progress/ }),
  ).toBeVisible();
  let panel = await controls(page);
  await panel
    .getByRole('button', { name: 'Advance step', exact: true })
    .click();
  await expect(dialog).toContainText('MiB processed');
  await evidence(page, info, 'paused-copy');
  await dialog
    .getByRole('button', { name: 'Continue in background', exact: true })
    .first()
    .click();
  await page.reload();
  await page
    .getByRole('button', { name: 'Open ownership transfer', exact: true })
    .click();
  await expect(dialog).toContainText('MiB processed');
  panel = await controls(page);
  for (let i = 0; i < 3; i++)
    await panel
      .getByRole('button', { name: 'Advance step', exact: true })
      .click();
  await expect(dialog).toContainText('Normal operations stay held.');
  await evidence(page, info, 'owned-held');
  for (let i = 0; i < 3; i++)
    await panel
      .getByRole('button', { name: 'Advance step', exact: true })
      .click();
  await expect(dialog).toContainText('Prowlarr is now Owned by Thelxinoe.');
  await evidence(page, info, 'transfer-completed');
});

test('failures expose real recovery choices and return to Connected', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  await scenario(page, 'failed-after', 'prowlarr', true);
  await page
    .getByRole('button', { name: 'Open transfer recovery', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  await expect(dialog.getByRole('alert')).toContainText(
    'The replacement reached production',
  );
  await dialog
    .getByRole('button', { name: 'Review recovery options', exact: true })
    .click();
  const plan = dialog.getByRole('region', { name: 'Recovery plan' });
  await plan.getByRole('radio', { name: /Return to the original/ }).check();
  await expect(
    plan.getByRole('button', { name: 'Apply recovery' }),
  ).toBeDisabled();
  for (const toggle of await plan.getByRole('switch').all())
    await toggle.check();
  await evidence(page, info, 'recovery-review');
  await plan
    .getByRole('button', { name: 'Apply recovery', exact: true })
    .click();
  await expect(dialog).toContainText(
    'Applying the approved recovery: Return to the original',
  );
  await (
    await controls(page)
  )
    .getByRole('button', { name: 'Advance step', exact: true })
    .click();
  await expect(dialog).toContainText('Prowlarr is Connected again.');
  await evidence(page, info, 'recovered-connected');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Take ownership', exact: true }),
  ).toBeVisible();
  await scenario(page, 'recovery-unavailable');
  await page.getByRole('button', { name: 'Open transfer recovery' }).click();
  await dialog.getByRole('button', { name: 'Review recovery options' }).click();
  await expect(
    dialog.getByRole('radio', { name: /Return to the original/ }),
  ).toBeDisabled();
  await expect(dialog).toContainText('The original container is missing');
});

test('link conflicts, retries and delayed disconnects update the real rows', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  let panel = await scenario(page, 'owned');
  await panel
    .getByLabel('Service link state', { exact: true })
    .selectOption('conflict');
  await panel
    .getByRole('button', { name: 'Apply controls', exact: true })
    .click();
  const row = page.getByLabel('Radarr connection', { exact: true });
  await expect(row).toContainText('Connection needs attention');
  await expect(
    row.getByRole('button', { name: 'Import existing', exact: true }),
  ).toBeVisible();
  await evidence(page, info, 'link-conflict');
  panel = await controls(page);
  await panel
    .getByLabel('Service link state', { exact: true })
    .selectOption('connected');
  await panel
    .getByRole('button', { name: 'Apply controls', exact: true })
    .click();
  await expect(row.getByRole('status').first()).toHaveText('Connected');
  // The floating scenario panel covers the row's buttons on a short page, so
  // close it while driving the row directly.
  await page
    .getByRole('button', { name: 'Service scenarios', exact: true })
    .click();
  await expect(panel).toBeHidden();
  await row.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(row.getByRole('status').first()).toHaveText(
    'Available to connect',
  );
  await row.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(row.getByRole('status').first()).toHaveText('Connected');
  panel = await controls(page);
  await panel
    .getByLabel('Service link state', { exact: true })
    .selectOption('cleanup');
  await panel
    .getByRole('button', { name: 'Apply controls', exact: true })
    .click();
  await expect(row).toContainText('Disconnect pending');
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await evidence(page, info, 'mobile-disconnect-pending');
});

test('an unconfigured manager rejects a failed connection and accepts a corrected one', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/radarr');
  const panel = await scenario(page, 'unconfigured', 'radarr');
  await panel
    .getByLabel('Connection result', { exact: true })
    .selectOption('unreachable');
  await panel.getByRole('button', { name: 'Apply controls' }).click();
  const setup = page.getByRole('form', { name: 'Connect existing Radarr' });
  await setup.getByLabel('Container').selectOption('container-radarr');
  await setup.getByLabel('API key', { exact: true }).fill('preview-api-key');
  await setup
    .getByRole('button', { name: 'Connect Radarr', exact: true })
    .click();
  await expect(
    page
      .getByText('The service could not be reached.', { exact: false })
      .first(),
  ).toBeVisible();
  await expect(
    page.getByRole('region', { name: 'Connected service' }),
  ).toHaveCount(0);
  await panel
    .getByLabel('Connection result', { exact: true })
    .selectOption('success');
  await panel.getByRole('button', { name: 'Apply controls' }).click();
  await setup
    .getByRole('button', { name: 'Connect Radarr', exact: true })
    .click();
  await expect(
    page.getByRole('region', { name: 'Connected service' }),
  ).toContainText('Verified');
  await evidence(page, info, 'manager-connected');
});

test('injected validation failures restore the original and automatic retries finish', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  const panel = await scenario(page, 'copy', 'prowlarr', true);
  await panel
    .getByLabel('Transfer outcome', { exact: true })
    .selectOption('before');
  await panel.getByRole('button', { name: 'Apply controls' }).click();
  await page
    .getByRole('button', { name: 'Open ownership transfer', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  await panel.getByRole('button', { name: 'Advance step' }).click();
  await expect(dialog).toContainText(
    'The replacement never reached production.',
  );
  await evidence(page, info, 'validation-failed');
  await dialog
    .getByRole('button', { name: 'Restore original', exact: true })
    .click();
  await expect(dialog).toContainText('Prowlarr is Connected again.');
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await scenario(page, 'failed-after', 'prowlarr', true);
  await page.getByRole('button', { name: 'Open transfer recovery' }).click();
  await dialog.getByRole('button', { name: 'Retry setup and checks' }).click();
  await panel
    .getByRole('checkbox', { name: 'Pause transfer progress' })
    .uncheck();
  await panel
    .getByLabel('Transfer outcome', { exact: true })
    .selectOption('success');
  await panel.getByRole('button', { name: 'Apply controls' }).click();
  await expect(dialog).toContainText('Prowlarr is now Owned by Thelxinoe.');
  await evidence(page, info, 'automatic-retry-complete');
});

test('completed transfers can be inspected directly and controls fit a small screen', async ({
  page,
}, info) => {
  await page.goto('/#admin/services/prowlarr');
  const panel = await scenario(page, 'complete');
  await panel
    .getByRole('button', { name: 'Inspect transfer', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Take ownership of Prowlarr',
  });
  await expect(dialog).toContainText('Prowlarr is now Owned by Thelxinoe.');
  await page.setViewportSize({ width: 390, height: 844 });
  await controls(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await evidence(page, info, 'mobile-completed-transfer');
});

for (const kind of [
  'seerr',
  'radarr',
  'sonarr',
  'lidarr',
  'bazarr',
  'prowlarr',
  'nzbget',
]) {
  test(`${kind} automatically finishes stopping the original after approval`, async ({
    page,
  }, info) => {
    const label =
      kind === 'nzbget' ? 'NZBGet' : kind[0].toUpperCase() + kind.slice(1);
    await page.goto(`/#admin/services/${kind}`);
    const panel = await scenario(page, 'review', kind);
    await expect(
      panel.getByRole('checkbox', { name: 'Pause transfer progress' }),
    ).not.toBeChecked();
    await panel
      .getByRole('button', { name: 'Close service scenarios' })
      .click();
    await page
      .getByRole('button', { name: 'Continue ownership review', exact: true })
      .click();
    const dialog = page.getByRole('dialog', {
      name: `Take ownership of ${label}`,
    });
    await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
    await dialog
      .getByRole('button', { name: "Retire the whole project's automation" })
      .click();
    await expect(
      dialog.getByRole('switch', {
        name: 'No automation or manual command will run against the project',
      }),
    ).toBeVisible();
    for (const toggle of await dialog.getByRole('switch').all())
      await toggle.check();
    await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
    await dialog
      .getByRole('switch', {
        name: 'I reviewed every change, and I understand the original stops',
      })
      .check();
    await dialog
      .getByRole('button', { name: 'Take ownership', exact: true })
      .click();
    const steps = dialog.getByRole('navigation', {
      name: 'Ownership transfer steps',
    });
    await expect(
      steps.getByRole('button', {
        name: /^5\. Stop the original: In progress/,
      }),
    ).toBeVisible();
    await expect(
      steps.getByRole('button', { name: /^5\. Stop the original: Done/ }),
    ).toBeVisible({ timeout: 6000 });
    await evidence(page, info, `${kind}-stop-completed`);
    if (kind === 'seerr') {
      await expect(dialog).toContainText('Seerr is now Owned by Thelxinoe.', {
        timeout: 30000,
      });
      await evidence(page, info, 'seerr-automatic-transfer-complete');
    }
  });
}
