// Uses only the isolated compose.acquisition.test.yaml deployment.
import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const origin = 'https://localhost:23443';
const browser = await chromium.launch();
const admin = await browser.newContext({ ignoreHTTPSErrors: true });
const user = await browser.newContext({ ignoreHTTPSErrors: true });
const headers = { 'X-Thelxinoe-Client': '1' };
async function api(context, path, method = 'GET', data) {
  const r = await context.request.fetch(`${origin}/api/v1${path}`, {
    method,
    headers,
    data,
  });
  if (!r.ok()) throw Error(`${path}: HTTP ${r.status()} ${await r.text()}`);
  return r.json();
}
const credentials = {
  username: 'admin',
  password: 'test-only long passphrase',
};
const results = [];
try {
  if ((await api(admin, '/setup')).setup_required) {
    await api(admin, '/setup', 'POST', credentials);
  } else await api(admin, '/auth/login', 'POST', credentials);
  if (
    !(await api(admin, '/users')).items.some(
      (u) => u.username === 'request-user',
    )
  )
    await api(admin, '/users', 'POST', {
      username: 'request-user',
      password: credentials.password,
      role: 'user',
    });
  await api(user, '/auth/login', 'POST', {
    username: 'request-user',
    password: credentials.password,
  });
  const page = await admin.newPage();
  await page.goto(`${origin}/?section=Settings`);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  for (const [kind, port, , version, root, term, external] of [
    ['radarr', 27878, 7878, 3, 'movies', 'tmdb:603', '603'],
    ['sonarr', 28989, 8989, 3, 'shows', 'tvdb:78874', '78874'],
    ['lidarr', 28686, 8686, 1, 'music', process.argv[2] ?? 'Discovery', null],
  ]) {
    const key = readFileSync(
      `.local/acquisition/${kind}/config.xml`,
      'utf8',
    ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
    async function manager(path, method = 'GET', body) {
      const r = await fetch(
        `http://127.0.0.1:${port}/api/v${version}/${path}`,
        {
          method,
          headers: { 'X-Api-Key': key, 'Content-Type': 'application/json' },
          body: body ? JSON.stringify(body) : undefined,
        },
      );
      if (!r.ok) throw Error(`${kind} ${path}: HTTP ${r.status}`);
      return r.json();
    }
    const profiles = await manager('qualityprofile');
    const metadata = kind === 'lidarr' ? await manager('metadataprofile') : [];
    if (!(await manager('rootfolder')).some((r) => r.path === `/data/${root}`))
      await manager('rootfolder', 'POST', {
        path: `/data/${root}`,
        name: 'Fixture music',
        defaultQualityProfileId: profiles[0].id,
        defaultMetadataProfileId: metadata[0]?.id,
        defaultMonitorOption: 'none',
        defaultTags: [],
      });
    const container = (
      await api(admin, '/admin/managers/containers')
    ).items.find((c) =>
      c.names.some((n) => n.includes(`acquisition-${kind}-`)),
    );
    if (!container) throw Error(`Missing ${kind} fixture container`);
    await page
      .getByLabel('Manager name', { exact: true })
      .fill(`Fixture ${kind}`);
    await page.getByLabel('Manager type').selectOption(kind);
    await page.getByLabel('Docker container').selectOption(container.id);
    await page.getByLabel('Manager API key', { exact: true }).fill(key);
    const registered = page.waitForResponse(
      (r) =>
        r.url().endsWith('/api/v1/admin/managers') &&
        r.request().method() === 'POST',
    );
    await page
      .getByRole('button', { name: 'Connect manager', exact: true })
      .click();
    const registration = await registered;
    if (!registration.ok())
      throw Error(`${kind} registration: ${await registration.text()}`);
    await expect(
      page.getByRole('button', {
        name: `Defaults for Fixture ${kind}`,
        exact: true,
      }),
    ).toBeVisible();
    await page
      .getByRole('button', {
        name: `Defaults for Fixture ${kind}`,
        exact: true,
      })
      .click();
    await page
      .getByLabel('Acquisition root folder')
      .selectOption(`/data/${root}`);
    await page
      .getByRole('button', { name: 'Save acquisition defaults', exact: true })
      .click();
    await expect(
      page
        .getByRole('status')
        .filter({ hasText: 'Acquisition defaults saved.' }),
    ).toBeVisible();
    const service = (await api(admin, '/admin/managers')).items.find(
      (s) => s.kind === kind,
    );
    const found = await api(
      user,
      `/acquisition/search?service_id=${service.id}&term=${encodeURIComponent(term)}`,
    );
    const selected = external
      ? found.items.find((r) => r.external_id === external)
      : found.items[0];
    if (!selected) throw Error(`No ${kind} search result`);
    const request = await api(user, '/acquisition/requests', 'POST', {
      service_id: service.id,
      external_id: selected.external_id,
    });
    if (request.state === 'pending')
      await api(admin, `/acquisition/requests/${request.id}`, 'POST', {
        action: 'approve',
      });
    let status;
    await expect
      .poll(
        async () => {
          status = (await api(admin, '/acquisition/requests')).items.find(
            (r) => r.id === request.id,
          );
          if (['failed', 'uncertain'].includes(status.state))
            throw Error(`${kind}: ${status.error}`);
          return status.state;
        },
        { timeout: 90000, intervals: [1000, 3000] },
      )
      .toBe('requested');
    results.push({
      kind,
      title: selected.title,
      state: status.state,
      manager_id: status.manager_id,
      availability: await api(
        user,
        `/acquisition/requests/${request.id}/status`,
      ),
    });
    console.log(JSON.stringify(results.at(-1)));
  }
  await page.getByRole('button', { name: 'Requests', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'All requests', exact: true }),
  ).toBeVisible();
  writeFileSync(
    '.local/acquisition-result.json',
    JSON.stringify(results, null, 2),
  );
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await browser.close();
}
