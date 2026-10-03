import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('first-time tools stay queued or installing until verified installation completes', async ({
  page,
}, info) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  const items = ['yt-dlp', 'deno', 'streamlink', 'ffmpeg'].map((id) => ({
    id,
    policy: 'inherit',
    effective_policy: 'notify',
    revision: 0,
    channel: id === 'yt-dlp' ? 'nightly' : id === 'deno' ? 'lts' : 'stable',
    pinned: false,
    installed: null as null | {
      id: string;
      candidate_id: string;
      version: string;
    },
    previous: null,
    candidate: null,
    checked_at: Math.floor(Date.now() / 1000),
    integrity_error: null,
    check_error: null,
    job: {
      id: id + '-bootstrap',
      action: 'bootstrap',
      manual: true,
      stage: 'queued',
      received: 0,
      total: 0,
      reason: null,
      error: null,
    },
  }));
  await page.route('**/api/v1/admin/tools', (route) =>
    route.fulfill({ json: { items, supported: true } }),
  );
  await page.goto('/');
  const tools = page.getByRole('region', { name: 'Server tools', exact: true });
  await expect(tools.getByRole('row')).toHaveCount(5);
  for (const row of await tools
    .getByRole('row')
    .all()
    .then((rows) => rows.slice(1))) {
    await expect(row.getByRole('status')).toContainText('Queued');
    await expect(
      row.getByRole('button', { name: 'Queued', exact: true }),
    ).toBeDisabled();
  }
  await tools.screenshot({ path: info.outputPath('tools-queued.png') });
  const ffmpeg = tools.getByRole('row', { name: /FFmpeg/ });
  for (const [stage, label] of [
    ['installing', 'Installing'],
    ['validating', 'Validating'],
  ]) {
    items[3].job.stage = stage;
    await page.evaluate(() =>
      window.dispatchEvent(new Event('thelxinoe-tools')),
    );
    await expect(ffmpeg.getByRole('status')).toContainText(label);
    await expect(
      ffmpeg.getByRole('button', { name: label, exact: true }),
    ).toBeDisabled();
  }
  for (const item of items) {
    item.installed = {
      id: item.id + '-verified',
      candidate_id: item.id + '-seed',
      version: '1.0',
    };
    item.job.stage = 'complete';
  }
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  for (const item of items)
    await expect(
      tools
        .getByRole('row', {
          name: new RegExp(
            item.id === 'ffmpeg'
              ? 'FFmpeg'
              : item.id === 'deno'
                ? 'Deno'
                : item.id === 'streamlink'
                  ? 'Streamlink'
                  : 'yt-dlp',
          ),
        })
        .getByRole('status'),
    ).toContainText('Up to date');
  await tools.screenshot({ path: info.outputPath('tools-installed.png') });
  expect(fixture.errors).toEqual([]);
});

test('server tools keep four inline rows through discovery, progress and eager policy saves', async ({
  page,
}, info) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  const items = ['yt-dlp', 'deno', 'streamlink', 'ffmpeg'].map((id) => ({
    id,
    policy: 'inherit',
    effective_policy: 'notify',
    channel: id === 'yt-dlp' ? 'nightly' : id === 'deno' ? 'lts' : 'stable',
    pinned: false,
    installed: { id: `${id}-old`, version: '1.0' },
    previous: null,
    candidate: { id: `${id}-new`, version: '1.1', notes_url: null },
    checked_at: Math.floor(Date.now() / 1000),
    check_error: null as string | null,
    integrity_error: null,
    job: null as null | {
      id: string;
      stage: string;
      received: number;
      total: number;
      error: string | null;
      reason: string | null;
    },
  }));
  const writes: { path: string; body: Record<string, unknown> }[] = [];
  await page.route('**/api/v1/admin/tools{,/**}', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (route.request().method() !== 'GET') {
      const body = route.request().postDataJSON();
      writes.push({ path, body });
      if (path.endsWith('/settings')) {
        await new Promise((resolve) => setTimeout(resolve, 250));
        Object.assign(
          items.find((item) => path.includes(`/${item.id}/`))!,
          body,
        );
      }
      return route.fulfill({ json: { accepted: true } });
    }
    return route.fulfill({ json: { items, supported: true } });
  });
  await page.goto('/');
  const tools = page.getByRole('region', { name: 'Server tools', exact: true });
  await expect(tools.getByRole('row')).toHaveCount(5);
  await expect(tools.locator('details')).toHaveCount(0);
  const row = tools.getByRole('row', { name: /yt-dlp/ });
  const installRequest = page.waitForRequest(
    (request) =>
      request.method() === 'POST' &&
      new URL(request.url()).pathname === '/api/v1/admin/tools/yt-dlp/install',
  );
  await row
    .getByRole('button', { name: 'Update yt-dlp to 1.1', exact: true })
    .click();
  expect((await installRequest).postDataJSON().candidate_id).toBe('yt-dlp-new');
  items[0].job = {
    id: 'install',
    stage: 'downloading',
    received: 1024,
    total: 4096,
    error: null,
    reason: null,
  };
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  await expect(row).toContainText('Downloading');
  const policy = row.getByRole('group', { name: 'yt-dlp update policy' });
  const inherit = policy.getByRole('button', { name: 'Inherit', exact: true });
  await expect(inherit).toHaveAttribute('aria-pressed', 'true');
  await policy.getByRole('button', { name: 'Automatic', exact: true }).click();
  await policy.getByRole('button', { name: 'Notify', exact: true }).click();
  await expect(
    policy.getByRole('button', { name: 'Notify', exact: true }),
  ).toBeEnabled();
  await expect.poll(() => items[0].policy).toBe('notify');
  await inherit.click();
  await expect(inherit).toHaveAttribute('aria-pressed', 'true');
  await expect.poll(() => items[0].policy).toBe('inherit');
  await expect(row.getByRole('spinbutton')).toHaveCount(0);
  const pin = row.getByRole('switch', { name: 'Pin yt-dlp', exact: true });
  await pin.check();
  await expect.poll(() => items[0].pinned).toBe(true);
  await pin.uncheck();
  await expect.poll(() => items[0].pinned).toBe(false);
  await expect(
    tools.getByRole('columnheader', { name: 'Available', exact: true }),
  ).toHaveCount(0);
  await expect(
    tools.getByRole('columnheader', { name: 'Pinned', exact: true }),
  ).toBeVisible();
  items[0].job = {
    ...items[0].job,
    stage: 'waiting',
    reason: 'Waiting for active downloads',
  };
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  await row
    .getByRole('button', { name: 'Update yt-dlp to 1.1', exact: true })
    .hover();
  await expect(row.getByRole('tooltip')).toContainText(
    'Waiting for active downloads',
  );
  await expect(tools.getByRole('row')).toHaveCount(5);
  await tools.screenshot({ path: info.outputPath('server-tools.png') });
  items[1].candidate.id = items[1].installed.id;
  Object.assign(items[1].installed, { candidate_id: items[1].candidate.id });
  items[1].check_error = 'Release service unavailable';
  items[1].checked_at -= 172800;
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  const deno = tools.getByRole('row', { name: /Deno/ });
  await deno
    .getByRole('button', { name: 'Check for Deno updates', exact: true })
    .hover();
  await expect(deno.getByRole('tooltip')).toContainText(
    'Release service unavailable',
  );
  await expect(deno.getByRole('tooltip')).toContainText('Last checked');
  await expect(deno).not.toContainText('Up to date');
  const checkRequest = page.waitForRequest(
    (request) =>
      request.method() === 'POST' &&
      new URL(request.url()).pathname === '/api/v1/admin/tools/deno/check',
  );
  await deno
    .getByRole('button', { name: 'Check for Deno updates', exact: true })
    .click();
  await checkRequest;
  items[1].check_error = null;
  items[1].checked_at = Math.floor(Date.now() / 1000);
  await page.evaluate(() => window.dispatchEvent(new Event('thelxinoe-tools')));
  await expect(deno.getByRole('status')).toContainText('Up to date');
  await tools.screenshot({ path: info.outputPath('server-tools-stale.png') });
  await info.attach('actions', {
    body: JSON.stringify(writes, null, 2),
    contentType: 'application/json',
  });
  expect(fixture.errors).toEqual([]);
});

test('server settings keep display defaults above update policy and tool tooltips outside the scrolling table', async ({
  page,
}, info) => {
  const fixture = await installUiFixture(page, {
    role: 'admin',
    settingsSection: 'server',
  });
  await page.route('**/api/v1/admin/tools', (route) =>
    route.fulfill({
      json: {
        supported: true,
        items: ['yt-dlp', 'deno', 'streamlink', 'ffmpeg'].map((id) => ({
          id,
          policy: 'inherit',
          effective_policy: 'notify',
          revision: 0,
          channel:
            id === 'yt-dlp' ? 'nightly' : id === 'deno' ? 'lts' : 'stable',
          pinned: false,
          installed: {
            id: `${id}-old`,
            candidate_id: `${id}-old`,
            version: '1.0',
          },
          previous: null,
          candidate: { id: `${id}-new`, version: '1.1', notes_url: null },
          checked_at: Math.floor(Date.now() / 1000),
          check_error: null,
          integrity_error: null,
          job: null,
        })),
      },
    }),
  );
  await page.goto('/');
  const display = page.getByRole('form', {
    name: 'Server display defaults',
    exact: true,
  });
  const policy = page.getByRole('form', {
    name: 'Server update settings',
    exact: true,
  });
  const tools = page.getByRole('region', { name: 'Server tools', exact: true });
  await expect(tools.getByRole('row')).toHaveCount(5);
  const displayBox = (await display.boundingBox())!;
  const policyBox = (await policy.boundingBox())!;
  const toolsBox = (await tools.boundingBox())!;
  expect(displayBox.y + displayBox.height).toBeLessThanOrEqual(policyBox.y);
  expect(toolsBox.y - policyBox.y - policyBox.height).toBeGreaterThanOrEqual(
    18,
  );
  await policy.getByRole('button', { name: 'Automatic', exact: true }).click();
  await expect(
    policy.getByRole('spinbutton', { name: 'Maintenance starts', exact: true }),
  ).toBeVisible();
  const ffmpeg = tools.getByRole('row', { name: /FFmpeg/ });
  const action = ffmpeg.getByRole('button', {
    name: 'Update FFmpeg + FFprobe to 1.1',
    exact: true,
  });
  await action.hover();
  const tooltip = ffmpeg.getByRole('tooltip');
  await expect(tooltip).toBeVisible();
  await expect(tooltip).toContainText('1.1');
  expect(
    await tooltip.evaluate((element) => element.matches(':popover-open')),
  ).toBe(true);
  await page.screenshot({
    path: info.outputPath('server-settings-desktop.png'),
  });
  await page.keyboard.press('Escape');
  await expect(tooltip).toBeHidden();
  await action.focus();
  await expect(tooltip).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await tools.scrollIntoViewIfNeeded();
  await action.hover();
  await expect(tooltip).toBeVisible();
  const bounds = (await tooltip.boundingBox())!;
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: info.outputPath('server-settings-mobile.png'),
  });
  expect(fixture.errors).toEqual([]);
});
