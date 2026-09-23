import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
try {
  const context = browser.contexts()[0];
  await context.setOffline(false);
  const page = context
    .pages()
    .find((p) => p.url().startsWith('http://tauri.localhost'));
  if (!page)
    throw new Error(
      'Launch the desktop app with WebView2 debugging on port 9223',
    );
  if (
    await page
      .getByRole('button', { name: 'Sign out', exact: true })
      .isVisible()
  )
    await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  await page.getByLabel('Server address').fill('http://127.0.0.1:18484');
  await page
    .getByRole('button', { name: 'Connect to server', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Welcome back' }),
  ).toBeVisible();
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Discover' })).toBeVisible();
  await expect(page.getByText('Connected', { exact: true })).toBeVisible();
  const stored = await page.evaluate(() => ({
    cookie: document.cookie,
    local: { ...localStorage },
    session: { ...sessionStorage },
  }));
  expect(stored.cookie).not.toContain('thelxinoe_session');
  const clientId = stored.local['thelxinoe-client-id'];
  if (clientId) expect(clientId).toMatch(/^[0-9a-f-]{36}$/);
  expect(
    Object.keys(stored.local).filter((key) => key !== 'thelxinoe-client-id'),
  ).toHaveLength(0);
  expect(Object.keys(stored.session)).toHaveLength(0);
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Discover' })).toBeVisible();
  await page
    .getByRole('navigation', { name: 'Main navigation' })
    .getByRole('button', { name: 'Movies', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Thelxinoe Fixture 2020', exact: true }),
  ).toBeVisible();
  execFileSync(
    'docker',
    ['compose', '-f', 'compose.test.yaml', 'stop', 'server'],
    { stdio: 'pipe', windowsHide: true },
  );
  await expect(page.getByText('Reconnecting', { exact: true })).toBeVisible({
    timeout: 20000,
  });
  execFileSync(
    'docker',
    ['compose', '-f', 'compose.test.yaml', 'up', '-d', '--wait'],
    { stdio: 'pipe', windowsHide: true },
  );
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({
    timeout: 20000,
  });
  await page.screenshot({ path: '.local/desktop-library.png' });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Devices', exact: true })
    .click();
  const row = page
    .locator('.row')
    .filter({ has: page.getByText('Windows desktop', { exact: true }) })
    .filter({ hasText: 'This device' });
  await expect(row).toHaveCount(1);
  await row.getByRole('button', { name: 'Revoke', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Sign in', exact: true }),
  ).toBeVisible();
  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Sign in', exact: true }),
  ).toBeVisible();
  await page.getByLabel('Server address').fill('http://127.0.0.1:8484');
  await page
    .getByRole('button', { name: 'Connect to server', exact: true })
    .click();
  console.log(
    'Desktop login, keyring persistence, catalog, event reconnect and revocation passed.',
  );
} finally {
  await browser.close();
}
