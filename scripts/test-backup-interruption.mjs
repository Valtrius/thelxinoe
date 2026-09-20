import { request, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec = promisify(execFile),
  controller = 'thelxinoe-operations-v4-controller-1';
const c = await request.newContext({
  baseURL: 'https://localhost:27443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function api(path, method = 'GET', data) {
  const r = await c.fetch('/api/v1' + path, { method, data });
  expect(r.status(), path).toBe(200);
  return r.json();
}
async function login() {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
}
async function records() {
  const result = await exec('docker', [
    'exec',
    controller,
    'curl',
    '-fsS',
    '--unix-socket',
    '/run/thelxinoe/controller.sock',
    'http://localhost/stack/backups',
  ]);
  return JSON.parse(result.stdout).items;
}
async function killAt(id, stage) {
  await expect
    .poll(
      async () => {
        const item = (await records()).find((r) => r.id === id);
        return item?.stage;
      },
      { timeout: 60000, intervals: [100] },
    )
    .toBe(stage);
  await exec('docker', ['kill', '--signal', 'KILL', controller]);
  await exec('docker', ['start', controller]);
}
async function recovered(id, stage) {
  await expect
    .poll(
      async () => {
        try {
          return (await records()).find((r) => r.id === id)?.stage;
        } catch {
          return 'restarting';
        }
      },
      { timeout: 120000, intervals: [1000] },
    )
    .toBe(stage);
  await expect
    .poll(
      async () => {
        try {
          return (await c.get('/api/v1/health')).status();
        } catch {
          return 0;
        }
      },
      { timeout: 60000 },
    )
    .toBe(200);
  await login();
}
try {
  await login();
  const passphrase = readFileSync(
    '.local/operations/backup-passphrase',
    'utf8',
  );
  const backup = readFileSync('.local/operations/backup-id', 'utf8').trim();
  const interrupted = await api('/admin/backups', 'POST', {
    passphrase,
    confirm: true,
  });
  await killAt(interrupted.id, 'snapshotting');
  await recovered(interrupted.id, 'failed');
  await api('/admin/settings', 'PUT', { timezone: 'Australia/Sydney' });
  await api(`/admin/backups/${backup}/restore`, 'POST', {
    passphrase,
    confirm: true,
  });
  await killAt(backup, 'restoring');
  await recovered(backup, 'restore-failed');
  expect((await api('/admin/settings')).timezone).toBe('Australia/Sydney');
  writeFileSync(
    '.local/backup-interruption-result.json',
    JSON.stringify(
      {
        passed: true,
        interruptedBackupRecovered: true,
        interruptedRestoreRolledBack: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Controller interruption during backup and restore recovered consistent state',
  );
} finally {
  await c.dispose();
}
