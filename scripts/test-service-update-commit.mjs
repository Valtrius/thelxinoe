import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 1000 },
});
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(
    'https://localhost:24443/api/v1' + path,
    { method, data, headers: { 'X-Thelxinoe-Client': '1' } },
  );
  expect(r.status(), path).toBe(200);
  return r.json();
}
async function wait(id, states) {
  let found;
  await expect
    .poll(
      async () => {
        found = (await api('/admin/service-updates')).items.find(
          (u) => u.id === id,
        );
        return states.includes(found?.state);
      },
      { timeout: 300000, intervals: [2000] },
    )
    .toBe(true);
  return found;
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const before = await api('/admin/stack');
  const service = before.items.find((s) => s.kind === 'radarr');
  const integration = before.provisions.find(
    (p) => p.id === service.id,
  ).service_id;
  const u = await api(
    '/admin/service-updates/preflight/' + service.id,
    'POST',
    {},
  );
  expect((await wait(u.id, ['ready', 'blocked'])).state).toBe('ready');
  await api('/admin/service-updates/' + u.id + '/activate', 'POST', {});
  expect(
    (
      await wait(u.id, [
        'committed',
        'blocked',
        'rolled-back',
        'runtime-failure',
        'recovery-required',
      ])
    ).state,
  ).toBe('committed');
  const after = await api('/admin/stack');
  const accepted = after.items.find((s) => s.id === service.id);
  expect(accepted.container_id).not.toBe(service.container_id);
  expect(accepted.drift).toBe(false);
  expect(accepted.running).toBe(true);
  expect(after.provisions.find((p) => p.id === service.id).service_id).toBe(
    integration,
  );
  expect(
    (await api('/admin/managers/' + integration + '/options')).roots.some(
      (r) => r.path === '/data/movies',
    ),
  ).toBe(true);
  const page = await context.newPage();
  await page.goto('https://localhost:24443');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  const panel = page.locator('section').filter({
    has: page.getByRole('heading', { name: 'Service updates', exact: true }),
  });
  await panel.getByLabel('Update policy for').selectOption(service.id);
  await panel.getByRole('combobox', { name: /^Policy/ }).selectOption('manual');
  await panel.getByLabel('Maintenance starts (UTC hour)').fill('22');
  await panel.getByLabel('Maintenance ends (UTC hour)').fill('3');
  await panel.getByRole('button', { name: 'Save update policy' }).click();
  await expect(panel.getByRole('status')).toContainText('Update policy saved');
  const saved = (await api('/admin/service-updates')).policies.find(
    (p) => p.service_id === service.id,
  );
  expect(saved.policy).toBe('manual');
  expect(saved.window_start).toBe(22);
  expect(saved.window_end).toBe(3);
  await panel.scrollIntoViewIfNeeded();
  await page.screenshot({ path: '.local/service-updates.png' });
  const result = {
    committed: true,
    integration_preserved: true,
    api_healthy: true,
    policy_ui: true,
    update_id: u.id,
    container_id: accepted.container_id,
  };
  writeFileSync(
    '.local/service-update-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(result);
} finally {
  await browser.close();
}
