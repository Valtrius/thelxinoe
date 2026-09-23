import { chromium, expect } from '@playwright/test';
import { existsSync, writeFileSync, unlinkSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
const project = process.env.THELXINOE_RELEASE_PROJECT || 'thelxinoe-release-v6';
const root = process.env.THELXINOE_RELEASE_STATE || '.local/releases-v6';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const r = await context.request
    .fetch('https://localhost:28443/api/v1' + path, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
      timeout: 5000,
    })
    .catch(() => {
      throw Error('Interrupted release transport unavailable');
    });
  if (!r.ok()) throw Error(`Release request failed: ${r.status()}`);
  return r.json();
}
function docker(args) {
  try {
    return execFileSync('docker', args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch {
    throw Error('Release fixture Docker operation failed');
  }
}
const hold = `${root}/server/hold-live-validation`;
const entered = `${root}/server/held-validation-entered`;
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  await api('/admin/product-update/policy', 'POST', {
    policy: 'notify',
    window_start: 0,
    window_end: 0,
  });
  await api('/admin/product-update/check', 'POST');
  const update = await api('/admin/product-update/prepare', 'POST');
  await expect
    .poll(
      async () => {
        try {
          return (await api('/admin/product-update')).controller.items.find(
            (u) => u.id === update.id,
          )?.stage;
        } catch {
          return 'reconnecting';
        }
      },
      { timeout: 180000, intervals: [1000] },
    )
    .toBe('ready');
  writeFileSync(hold, 'hold the migrated candidate before activation');
  docker(['stop', 'thelxinoe-release-registry']);
  await api(`/admin/product-update/${update.id}/activate`, 'POST', {
    confirm: true,
  });
  await expect
    .poll(() => existsSync(entered), { timeout: 90000, intervals: [100] })
    .toBe(true);
  docker(['restart', '--time', '0', `${project}-controller-1`]);
  await expect
    .poll(
      async () => {
        try {
          return (await api('/admin/product-update')).controller.items.find(
            (u) => u.id === update.id,
          )?.stage;
        } catch {
          return 'reconnecting';
        }
      },
      { timeout: 180000, intervals: [1000] },
    )
    .toBe('recovered');
  expect((await api('/health')).version).toBe('0.1.0');
  expect((await api('/admin/diagnostics')).database_ok).toBe(true);
  expect(existsSync(entered)).toBe(false);
  expect(
    JSON.parse(docker(['inspect', 'thelxinoe-release-registry']))[0].State
      .Running,
  ).toBe(false);
  writeFileSync(
    '.local/release-interruption-result.json',
    JSON.stringify(
      {
        passed: true,
        registryOffline: true,
        controllerKilledAfterForwardMigration: true,
        originalStateRecovered: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Controller termination after forward migration recovered original state with the registry offline',
  );
} finally {
  docker(['start', 'thelxinoe-release-registry']);
  if (existsSync(hold)) unlinkSync(hold);
  await browser.close();
}
