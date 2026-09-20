import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { randomBytes } from 'node:crypto';
import { execFileSync } from 'node:child_process';
const origin = 'https://localhost:27443';
const browser = await chromium.launch();
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 1000 },
});
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(origin + '/api/v1' + path, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout: 120000,
  });
  const value = await r.json();
  if (!r.ok()) throw Error(`${path}: ${r.status()} ${value.error?.message}`);
  return value;
}
const credentials = {
  username: 'admin',
  password: 'test-only long passphrase',
};
async function login() {
  await api('/auth/login', 'POST', credentials);
}
async function waitBackup(id, stage) {
  await expect
    .poll(
      async () => {
        try {
          const item = (await api('/admin/backups')).items.find(
            (i) => i.id === id,
          );
          if (
            ['failed', 'restore-failed', 'recovery-required'].includes(
              item?.stage,
            )
          )
            throw Error(item.error);
          return item?.stage;
        } catch (e) {
          if (
            String(e).includes('502') ||
            String(e).includes('ECONNRESET') ||
            String(e).includes('Unexpected token') ||
            String(e).includes('Unexpected end of JSON')
          )
            return 'offline';
          throw e;
        }
      },
      { timeout: 240000, intervals: [2000] },
    )
    .toBe(stage);
}
try {
  if ((await api('/setup')).setup_required)
    await api('/setup', 'POST', {
      ...credentials,
      setup_token: readFileSync(
        '.local/operations-v4/server/secrets/setup-token',
        'utf8',
      ).trim(),
    });
  else await login();
  let managed = (await api('/admin/stack')).items.find(
    (s) => s.kind === 'radarr',
  );
  if (!managed) {
    await api('/admin/stack/install', 'POST', {
      kind: 'radarr',
      host_port: 48787,
      native_url: 'http://127.0.0.1:48787',
    });
    await expect
      .poll(
        async () => {
          const data = await api('/admin/stack');
          const failed = data.provisions.find(
            (p) => p.kind === 'radarr' && p.state === 'failed',
          );
          if (failed) throw Error(failed.error);
          managed = data.items.find(
            (s) => s.kind === 'radarr' && s.phase === 'active',
          );
          return !!managed;
        },
        { timeout: 240000, intervals: [2000] },
      )
      .toBe(true);
  }
  expect(managed.id).toMatch(/^[a-f0-9-]{36}$/);
  const controller = 'thelxinoe-operations-v4-controller-1';
  const sentinel = `/var/lib/thelxinoe/deployment/services/${managed.id}/appdata/backup-fixture`;
  const setSentinel = (value) =>
    execFileSync('docker', ['exec', '-i', controller, 'tee', sentinel], {
      input: value,
      stdio: ['pipe', 'ignore', 'pipe'],
    });
  setSentinel('before-backup');
  await expect
    .poll(
      async () =>
        (await api('/admin/jobs')).items.filter((j) => j.state === 'running')
          .length,
      { timeout: 180000, intervals: [2000] },
    )
    .toBe(0);
  const before = await api('/admin/settings');
  await api('/admin/settings', 'PUT', { timezone: 'Europe/Paris' });
  const secret = randomBytes(24).toString('hex');
  mkdirSync('.local/operations', { recursive: true });
  // A private fixture recovery input, never printed or tracked.
  writeFileSync('.local/operations/backup-passphrase', secret);
  const created = await api('/admin/backups', 'POST', {
    passphrase: secret,
    confirm: true,
  });
  writeFileSync('.local/operations/backup-id', created.id);
  await waitBackup(created.id, 'complete');
  await api('/admin/settings', 'PUT', { timezone: 'Asia/Tokyo' });
  setSentinel('after-backup');
  const rejected = await context.request.post(
    origin + `/api/v1/admin/backups/${created.id}/restore`,
    {
      headers: { 'X-Thelxinoe-Client': '1' },
      data: { passphrase: 'this passphrase is incorrect', confirm: true },
    },
  );
  expect(rejected.status()).toBe(409);
  expect((await api('/admin/settings')).timezone).toBe('Asia/Tokyo');
  await api(`/admin/backups/${created.id}/restore`, 'POST', {
    passphrase: secret,
    confirm: true,
  });
  await waitBackup(created.id, 'restored');
  expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
  expect(
    execFileSync('docker', ['exec', controller, 'cat', sentinel], {
      encoding: 'utf8',
    }),
  ).toBe('before-backup');
  const manager = (await api('/admin/managers')).items.find(
    (s) => s.kind === 'radarr',
  );
  expect(manager).toBeTruthy();
  await expect
    .poll(
      async () => {
        const response = await context.request.post(
          origin + `/api/v1/admin/managers/${manager.id}/test`,
          { headers: { 'X-Thelxinoe-Client': '1' } },
        );
        return response.status();
      },
      { timeout: 90000, intervals: [2000] },
    )
    .toBe(200);
  const diagnostic = await api('/admin/diagnostics');
  expect(diagnostic.database_ok).toBe(true);
  expect(JSON.stringify(diagnostic)).not.toContain(secret);
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Server', exact: true })
    .click();
  await expect(
    page.getByRole('region', { name: 'Administration overview' }),
  ).toBeVisible();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Backups', exact: true })
    .click();
  await expect(
    page.getByRole('region', { name: 'Backups', exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: '.local/operations.png', fullPage: true });
  await api('/admin/settings', 'PUT', { timezone: before.timezone });
  writeFileSync(
    '.local/operations-result.json',
    JSON.stringify(
      {
        passed: true,
        backup: created.id,
        wrongPassphraseRejected: true,
        settingsRestored: true,
        managedAppdataRestored: true,
        diagnosticsRedacted: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Encrypted backup, rejected passphrase, live state restoration and admin UI passed',
  );
} finally {
  await browser.close();
}
