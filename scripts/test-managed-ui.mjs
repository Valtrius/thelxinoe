import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const b = await chromium.launch();
const c = await b.newContext({
  ignoreHTTPSErrors: true,
  viewport: { width: 1440, height: 1000 },
});
const page = await c.newPage();
const base = 'https://localhost:24443';
async function api(p, m = 'GET', data, expected = 200) {
  const r = await c.request.fetch(base + '/api/v1' + p, {
    method: m,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  expect(r.status()).toBe(expected);
  return r.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  await page.goto(base);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  for (const [kind, root] of [
    ['radarr', '/media/movies'],
    ['sonarr', '/media/tv'],
    ['lidarr', '/media/music'],
  ]) {
    await page
      .getByRole('button', {
        name: 'Defaults for Managed ' + kind,
        exact: true,
      })
      .click();
    await expect(page.getByLabel('Acquisition root folder')).toHaveValue(root);
    await page
      .getByRole('button', { name: 'Save acquisition defaults', exact: true })
      .click();
    await expect(
      page.getByRole('button', {
        name: 'Save acquisition defaults',
        exact: true,
      }),
    ).not.toBeVisible();
    const manager = (await api('/admin/managers')).items.find(
      (m) => m.kind === kind,
    );
    await expect
      .poll(async () =>
        (await api('/admin/managers/' + manager.id + '/options')).roots.some(
          (r) => r.path === root && r.id > 0,
        ),
      )
      .toBe(true);
  }
  const stack = await api('/admin/stack');
  const radarr = stack.items.find((s) => s.kind === 'radarr');
  const raw = JSON.parse(
    execFileSync('docker', ['inspect', radarr.container_id], {
      encoding: 'utf8',
    }),
  )[0];
  expect(raw.Config.Labels['app.thelxinoe.deployment']).toBe(
    stack.deployment_id,
  );
  try {
    execFileSync('docker', ['update', '--restart=no', radarr.container_id], {
      stdio: 'pipe',
    });
    expect(
      (await api('/admin/stack')).items.find((s) => s.id === radarr.id).drift,
    ).toBe(true);
    await api(
      '/admin/stack/' + radarr.id + '/action',
      'POST',
      { action: 'restart' },
      409,
    );
  } finally {
    execFileSync(
      'docker',
      ['update', '--restart=unless-stopped', radarr.container_id],
      { stdio: 'pipe' },
    );
  }
  expect(
    (await api('/admin/stack')).items.find((s) => s.id === radarr.id).drift,
  ).toBe(false);
  await page
    .getByRole('heading', { name: 'Managed services', exact: true })
    .scrollIntoViewIfNeeded();
  await page.screenshot({ path: '.local/managed-settings.png' });
  writeFileSync(
    '.local/managed-ui-result.json',
    JSON.stringify({
      canonical_roots: ['movies', 'tv', 'music'],
      drift_blocked: true,
      restored: true,
    }),
  );
  console.log('Canonical roots and drift rejection passed.');
} finally {
  await b.close();
}
