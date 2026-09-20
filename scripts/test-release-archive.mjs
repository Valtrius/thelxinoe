import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const response = await context.request
    .fetch(`https://localhost:28443/api/v1${path}`, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
    })
    .catch(() => {
      throw Error('Test transport unavailable');
    });
  if (!response.ok())
    throw Error(
      `${path}: ${response.status()} ${(await response.text()).slice(0, 250)}`,
    );
  return response.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const archive = (await api('/admin/backups')).items.find(
    (r) =>
      r.stage === (process.argv.includes('--verify') ? 'restored' : 'complete'),
  );
  expect(archive).toBeTruthy();
  if (!process.argv.includes('--verify')) {
    expect((await api('/health')).version).toBe('0.2.0');
    await api(`/admin/backups/${archive.id}/restore`, 'POST', {
      confirm: true,
      passphrase: 'private release test archive passphrase',
    });
  }
  await expect
    .poll(
      async () => {
        try {
          return (await api('/admin/backups')).items.find(
            (r) => r.id === archive.id,
          )?.stage;
        } catch {
          try {
            await api('/auth/login', 'POST', {
              username: 'admin',
              password: 'test-only long passphrase',
            });
          } catch {
            /* Recovery may still be starting. */
          }
          return 'reconnecting';
        }
      },
      { timeout: 240000, intervals: [1000] },
    )
    .toBe('restored');
  expect((await api('/health')).version).toBe('0.1.0');
  expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
  expect((await api('/admin/diagnostics')).database_ok).toBe(true);
  writeFileSync(
    '.local/release-archive-result.json',
    JSON.stringify({
      passed: true,
      earlierGenerationRestored: true,
      archive: archive.id,
    }),
  );
  console.log(
    'Encrypted archive restored the earlier accepted release and database',
  );
} finally {
  await browser.close();
}
