import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { setTimeout } from 'node:timers/promises';
const endpoint = 'http://127.0.0.1:9224';
async function connect() {
  const browser = await chromium.connectOverCDP(endpoint);
  const page = browser
    .contexts()[0]
    .pages()
    .find((p) => p.url().startsWith('http://tauri.localhost'));
  if (!page) throw Error('The isolated native release fixture is not open');
  return { browser, page };
}
let { browser, page } = await connect();
const native = (command, args = {}) =>
  page.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
const metadataPath = '.local/releases/channel/windows-x64.json';
const artifactPath = '.local/releases/channel/setup.exe';
const metadata = readFileSync(metadataPath);
const artifact = readFileSync(artifactPath);
try {
  await native('change_server', { value: 'http://127.0.0.1:19494' });
  const login = await native('backend_request', {
    path: '/auth/login',
    method: 'POST',
    body: { username: 'admin', password: 'test-only long passphrase' },
  });
  expect(login.status).toBe(200);
  expect(login.body.token).toBeUndefined();
  const checked = await native('desktop_update_check');
  expect(checked.installed).toBe('0.1.0');
  expect(checked.release.version).toBe('0.2.0');
  await page.reload();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('button', { name: 'Check desktop release', exact: true })
    .click();
  await expect(
    page
      .getByRole('region', { name: 'Desktop updates' })
      .getByText('Version 0.2.0'),
  ).toBeVisible();
  const mismatched = JSON.parse(metadata);
  mismatched.platforms['windows-x86_64'].signature =
    'mismatched-test-signature';
  writeFileSync(metadataPath, JSON.stringify(mismatched));
  let rejection = '';
  try {
    await native('desktop_update_install');
  } catch (error) {
    rejection = String(error);
  }
  expect(rejection).toContain('does not match the signed product release');
  writeFileSync(metadataPath, metadata);
  const corrupted = Buffer.from(artifact);
  corrupted[corrupted.length - 20] ^= 1;
  writeFileSync(artifactPath, corrupted);
  rejection = '';
  try {
    await native('desktop_update_install');
  } catch (error) {
    rejection = String(error);
  }
  expect(rejection).toContain('signature verification failed');
  writeFileSync(artifactPath, artifact);
  await page
    .getByRole('region', { name: 'Desktop updates' })
    .screenshot({ path: '.local/native-update-before.png' });
  await page
    .getByRole('button', {
      name: 'Download and install desktop update',
      exact: true,
    })
    .click();
  let installed = '';
  for (let i = 0; i < 120; i++) {
    await setTimeout(1000);
    try {
      if (page.isClosed() || !browser.isConnected())
        ({ browser, page } = await connect());
      installed = (await native('desktop_update_check')).installed;
      if (installed === '0.2.0') break;
    } catch {
      /* Installer closes and reopens the native WebView. */
    }
  }
  expect(installed).toBe('0.2.0');
  const authenticated = await native('backend_request', {
    path: '/auth/me',
    method: 'GET',
    body: null,
  });
  expect(authenticated.status).toBe(200);
  await page.screenshot({ path: '.local/native-update-after.png' });
  writeFileSync(
    '.local/native-update-result.json',
    JSON.stringify(
      {
        passed: true,
        metadataMismatchRejected: true,
        corruptSignatureRejected: true,
        installedVersion: installed,
        deviceCredentialPreserved: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Native Windows update rejected mismatched/corrupt artifacts and installed the signed 0.2.0 release',
  );
} finally {
  writeFileSync(metadataPath, metadata);
  writeFileSync(artifactPath, artifact);
  // Closing the CDP connection must not close the application under test.
  await browser.close().catch(() => {});
}
