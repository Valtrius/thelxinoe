import { chromium, expect } from '@playwright/test';
import { writeFileSync, unlinkSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const origin = 'https://localhost:28443';
const project = process.env.THELXINOE_RELEASE_PROJECT || 'thelxinoe-release-v6';
const dataRoot = process.env.THELXINOE_RELEASE_STATE || '.local/releases-v6';
const archiveTest = process.env.THELXINOE_RELEASE_ARCHIVE === '1';
const browser = await chromium.launch();
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 1000 },
});
const credentials = {
  username: 'admin',
  password: 'test-only long passphrase',
};
async function api(path, method = 'GET', data) {
  const r = await context.request
    .fetch(origin + '/api/v1' + path, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
      timeout: 120000,
    })
    .catch(() => {
      throw Error('Release test transport unavailable');
    });
  if (!r.ok())
    throw Error(`${path}: ${r.status()} ${(await r.text()).slice(0, 350)}`);
  return r.json();
}
async function wait(id, stage) {
  await expect
    .poll(
      async () => {
        try {
          const u = (await api('/admin/product-update')).controller.items.find(
            (u) => u.id === id,
          );
          if (
            ['blocked', 'recovery-required', 'runtime-failure'].includes(
              u?.stage,
            ) &&
            u.stage !== stage
          )
            throw Error(u.error);
          return u?.stage;
        } catch (e) {
          if (
            /502|ECONNRESET|Unexpected token|ECONNREFUSED|transport unavailable/.test(
              String(e),
            )
          )
            return 'reconnecting';
          throw e;
        }
      },
      { timeout: 300000, intervals: [1000] },
    )
    .toBe(stage);
}
function controller(path) {
  return JSON.parse(
    execFileSync(
      'docker',
      [
        'exec',
        `${project}-controller-1`,
        'curl',
        '-fsS',
        '--unix-socket',
        '/run/thelxinoe/controller.sock',
        'http://localhost' + path,
      ],
      { encoding: 'utf8' },
    ),
  );
}
const marker = `${dataRoot}/server/fail-live-validation`;
async function waitBackup(id, stage) {
  await expect
    .poll(
      async () => {
        try {
          return (await api('/admin/backups')).items.find((u) => u.id === id)
            ?.stage;
        } catch {
          return 'reconnecting';
        }
      },
      { timeout: 300000, intervals: [1000] },
    )
    .toBe(stage);
}
try {
  if ((await api('/setup')).setup_required)
    await api('/setup', 'POST', credentials);
  else await api('/auth/login', 'POST', credentials);
  await api('/admin/product-update/policy', 'POST', {
    policy: 'manual',
    window_start: 0,
    window_end: 0,
  });
  await api('/admin/product-update/check', 'POST');
  expect((await api('/admin/product-update')).release.version).toBe('0.2.0');
  await api('/admin/settings', 'PUT', { timezone: 'Europe/Paris' });
  const archive = archiveTest
    ? await api('/admin/backups', 'POST', {
        confirm: true,
        passphrase: 'private release test archive passphrase',
      })
    : null;
  if (archive) await waitBackup(archive.id, 'complete');
  const failed = await api('/admin/product-update/prepare', 'POST');
  await wait(failed.id, 'ready');
  writeFileSync(marker, 'fail after migrating real state');
  await api(`/admin/product-update/${failed.id}/activate`, 'POST', {
    confirm: true,
  });
  await wait(failed.id, 'recovered');
  expect(
    (await api('/admin/product-update')).controller.items.find(
      (u) => u.id === failed.id,
    ).error,
  ).toContain('Isolated worker failed its contract');
  expect((await api('/health')).version).toBe('0.1.0');
  const diagnostic = await api('/admin/diagnostics');
  expect(diagnostic.database_ok).toBe(true);
  expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
  unlinkSync(marker);
  if (archive) {
    const next = await api('/admin/product-update/prepare', 'POST');
    await wait(next.id, 'ready');
    await api(`/admin/product-update/${next.id}/activate`, 'POST', {
      confirm: true,
    });
    await wait(next.id, 'committed');
    await api('/admin/settings', 'PUT', { timezone: 'Asia/Tokyo' });
    await api(`/admin/backups/${archive.id}/restore`, 'POST', {
      confirm: true,
      passphrase: 'private release test archive passphrase',
    });
    await waitBackup(archive.id, 'restored');
    expect((await api('/health')).version).toBe('0.1.0');
    expect(controller('/health').version).toBe('0.1.0');
    expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
  }
  const update = await api('/admin/product-update/prepare', 'POST');
  await wait(update.id, 'ready');
  await api(`/admin/product-update/${update.id}/activate`, 'POST', {
    confirm: true,
  });
  await wait(update.id, 'committed');
  expect((await api('/health')).version).toBe('0.2.0');
  expect(controller('/health').version).toBe('0.2.0');
  await api('/admin/settings', 'PUT', { timezone: 'Asia/Tokyo' });
  await api(`/admin/product-update/${update.id}/recover`, 'POST', {
    confirm: true,
  });
  await wait(update.id, 'restored');
  expect((await api('/health')).version).toBe('0.1.0');
  expect(controller('/health').version).toBe('0.1.0');
  expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Updates', exact: true })
    .click();
  await page
    .getByRole('region', { name: 'Product updates', exact: true })
    .screenshot({ path: '.local/product-updates.png' });
  writeFileSync(
    '.local/product-release-result.json',
    JSON.stringify(
      {
        passed: true,
        failedForwardMigrationRecovered: true,
        signedReleaseActivated: true,
        controllerHandoff: true,
        explicitOfflineStateRestored: true,
        earlierGenerationArchiveRestored: !!archive,
        update: update.id,
      },
      null,
      2,
    ),
  );
  console.log(
    'Signed release, failed forward migration, controller handoff and full state recovery passed',
  );
} finally {
  await browser.close();
}
