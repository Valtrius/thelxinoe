import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, unlinkSync } from 'node:fs';
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
  });
  expect(r.status(), path).toBe(200);
  return r.json();
}
let fixture;
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const page = await context.newPage();
  await page.goto(origin);
  await page.getByRole('link', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('link', { name: 'People', exact: true })
    .click();
  const name = 'cleanup-' + Date.now();
  await page.getByLabel('Username', { exact: true }).fill(name);
  await page
    .getByLabel('Password', { exact: true })
    .fill('temporary test passphrase');
  await page.getByRole('button', { name: 'Add user', exact: true }).click();
  const person = page.locator('.person').filter({ hasText: name });
  await expect(person).toBeVisible();
  const guest = await browser.newContext({ ignoreHTTPSErrors: true });
  const login = await guest.request.post(origin + '/api/v1/auth/login', {
    headers: { 'X-Thelxinoe-Client': '1' },
    data: { username: name, password: 'temporary test passphrase' },
  });
  expect(login.status()).toBe(200);
  expect(
    (await guest.request.get(origin + '/api/v1/admin/diagnostics')).status(),
  ).toBe(403);
  mkdirSync('.local/operations-v4/data/broken', { recursive: true });
  fixture = '.local/operations-v4/data/broken/invalid TV.mp4';
  writeFileSync(fixture, 'deliberately invalid test media');
  const roots = (await api('/catalog/roots')).items;
  const root =
    roots.find((r) => r.path === '/media/broken') ??
    (await api('/catalog/roots', 'POST', {
      name: 'Failure fixture',
      kind: 'shows',
      path: '/media/broken',
    }));
  await api(`/catalog/roots/${root.id}/scan`, 'POST');
  const button = page.getByRole('link', { name: 'Settings', exact: true });
  await expect(button.locator('[data-attention-severity]')).toBeVisible({
    timeout: 90000,
  });
  expect(
    (await (await guest.request.get(origin + '/api/v1/me/attention')).json())
      .items,
  ).toHaveLength(0);
  const nav = page.getByRole('navigation', { name: 'Settings navigation' });
  await nav.getByRole('link', { name: 'Activity', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Dismiss failure', exact: true }).first(),
  ).toBeVisible();
  const failures = page.getByRole('button', {
    name: 'Dismiss failure',
    exact: true,
  });
  while (await failures.count()) {
    const count = await failures.count();
    await failures.first().click();
    await expect(failures).toHaveCount(count - 1);
  }
  await expect(
    nav
      .getByRole('link', { name: 'Activity', exact: true })
      .locator('[data-attention-severity]'),
  ).toHaveCount(0);
  await nav.getByRole('link', { name: 'People', exact: true }).click();
  await person
    .getByRole('button', { name: 'Manage user', exact: true })
    .click();
  await person
    .getByLabel(`Type ${name} to confirm deletion`, { exact: true })
    .fill(name);
  await person
    .getByRole('button', { name: 'Delete user and personal data', exact: true })
    .click();
  await expect(person).toHaveCount(0);
  expect((await guest.request.get(origin + '/api/v1/auth/me')).status()).toBe(
    401,
  );
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('link', { name: 'Server', exact: true })
    .click();
  const dashboard = page.getByRole('region', {
    name: 'Administration overview',
    exact: true,
  });
  await dashboard.screenshot({ path: '.local/admin-overview.png' });
  await guest.close();
  writeFileSync(
    '.local/admin-ui-result.json',
    JSON.stringify(
      {
        passed: true,
        realtimeAttention: true,
        privateNotices: true,
        createdAndDeletedUser: true,
        revokedDeletedSession: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Browser attention dots, private state and user administration passed',
  );
} finally {
  if (fixture) unlinkSync(fixture);
  await browser.close();
}
