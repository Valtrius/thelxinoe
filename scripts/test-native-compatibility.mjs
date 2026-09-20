import { chromium, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { writeFileSync } from 'node:fs';

// Requires the isolated app.thelxinoe.releasetest installation, never the normal desktop profile.
const browser = await chromium.connectOverCDP('http://127.0.0.1:9224');
const page = browser
  .contexts()[0]
  .pages()
  .find((p) => p.url().startsWith('http://tauri.localhost'));
if (!page)
  throw Error('Open the isolated native release test application first');
const native = (command, args = {}) =>
  page.evaluate(
    ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
    { command, args },
  );
const original = await native('server_url');
expect(original).toBe('http://127.0.0.1:19494');
const server = createServer((request, response) => {
  response.setHeader('Content-Type', 'application/json');
  response.end(
    JSON.stringify(
      request.url === '/api/v1/health'
        ? {
            status: 'ok',
            version: '99.0.0',
            api_version: 99,
            api_min: 99,
            api_max: 99,
          }
        : { envelope: null },
    ),
  );
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
try {
  await native('change_server', {
    value: `http://127.0.0.1:${server.address().port}`,
  });
  await page.reload();
  await expect(
    page.getByRole('heading', { name: 'Update required' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Check desktop release', exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel('Server address')).toBeVisible();
  await page.screenshot({ path: '.local/native-update-required.png' });
  writeFileSync(
    '.local/native-compatibility-result.json',
    JSON.stringify({
      passed: true,
      incompatibleServerBlocked: true,
      desktopUpdaterAvailable: true,
    }),
  );
  console.log(
    'Installed Windows app blocks incompatible servers and exposes desktop updates',
  );
} finally {
  await native('change_server', { value: original });
  const login = await native('backend_request', {
    path: '/auth/login',
    method: 'POST',
    body: { username: 'admin', password: 'test-only long passphrase' },
  });
  expect(login.status).toBe(200);
  await page.reload();
  await browser.close();
  await new Promise((resolve) => server.close(resolve));
}
