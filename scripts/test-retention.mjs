// Destructive validation restricted to the generated acquisition fixture.
import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(
    'https://localhost:23443/api/v1' + path,
    {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
      timeout: 120000,
    },
  );
  const body = await response.json();
  if (!response.ok())
    throw Error(
      `${path}: HTTP ${response.status()} ${body.error?.message ?? ''}`,
    );
  return body;
}
async function radarr(path) {
  const key = readFileSync(
    '.local/acquisition/radarr/config.xml',
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const response = await fetch('http://localhost:27878/api/v3/' + path, {
    headers: { 'X-Api-Key': key },
  });
  if (!response.ok) throw Error('Fixture Radarr HTTP ' + response.status);
  return response.json();
}
let original;
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const settings = await api('/admin/retention');
  original = settings.policies.find((p) => p.domain === 'movies');
  const user = settings.users.find((u) => u.username === 'admin');
  const roots = (await api('/catalog/roots')).items;
  const root = roots.find((r) => r.path === '/data/movies');
  expect(root).toBeTruthy();
  const job = (await api(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
  await expect
    .poll(
      async () =>
        (await api('/admin/jobs')).items.find((j) => j.id === job)?.state,
      { timeout: 60000 },
    )
    .toBe('complete');
  const movie = (await api('/catalog?kind=movie')).items.find(
    (m) => m.available && m.title.toLowerCase().includes('matrix'),
  );
  expect(movie, 'Import the generated Matrix fixture first').toBeTruthy();
  await api('/admin/managers/reconcile', 'POST');
  const files = (await api('/admin/managers/bindings')).items.filter((f) =>
    f.path.startsWith('/data/movies/'),
  );
  expect(files).toHaveLength(1);
  expect(files[0].path).toContain('FIXTURE2');
  expect(files[0].ownership).toBe('managed');
  const before = (await radarr('movie')).find((m) => m.tmdbId === 603);
  const policy = {
    ...original,
    enabled: true,
    grace_seconds: 86400,
    trigger_users: [user.id],
  };
  await api('/admin/retention/policy/movies', 'POST', policy);
  await api(`/catalog/${movie.id}/state`, 'PUT', { watched: true });
  await api('/admin/retention/evaluate', 'POST');
  let candidate = (await api('/admin/retention')).items.find(
    (c) => c.media_id === movie.id && c.state === 'pending',
  );
  expect(candidate).toBeTruthy();
  await api(`/admin/retention/${candidate.id}/keep`, 'POST');
  expect((await radarr(`movie/${before.id}`)).hasFile).toBe(true);
  await api('/admin/retention/evaluate', 'POST');
  expect(
    (await api('/admin/retention')).items.some(
      (c) => c.media_id === movie.id && c.state === 'pending',
    ),
  ).toBe(false);
  await api(`/admin/media/${movie.id}/keep`, 'PUT', { keep: false });
  await api('/admin/retention/policy/movies', 'POST', policy);
  await api('/admin/retention/evaluate', 'POST');
  candidate = (await api('/admin/retention')).items.find(
    (c) => c.media_id === movie.id && c.state === 'pending',
  );
  await api(`/admin/retention/${candidate.id}/delete`, 'POST');
  const after = await radarr(`movie/${before.id}`);
  expect(after.hasFile).toBe(false);
  expect(after.monitored).toBe(false);
  expect((await radarr('exclusions')).some((e) => e.tmdbId === 603)).toBe(true);
  await api('/admin/retention/policy/movies', 'POST', {
    ...policy,
    enabled: false,
  });
  const request = (await api('/acquisition/requests')).items.find(
    (r) => r.kind === 'radarr' && r.manager_id === before.id,
  );
  await api(`/acquisition/requests/${request.id}`, 'POST', {
    action: 'reacquire',
  });
  await expect
    .poll(
      async () =>
        (await api('/acquisition/requests')).items.find(
          (r) => r.id === request.id,
        )?.state,
      { timeout: 120000 },
    )
    .toBe('requested');
  expect((await radarr(`movie/${before.id}`)).monitored).toBe(true);
  expect((await radarr('exclusions')).some((e) => e.tmdbId === 603)).toBe(
    false,
  );
  const page = await context.newPage();
  await page.goto('https://localhost:23443');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Watched media retention', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('heading', { name: 'Watched media retention', exact: true })
    .scrollIntoViewIfNeeded();
  await expect(
    page.getByRole('button', { name: 'Save movie policy' }),
  ).toBeEnabled();
  await page.waitForLoadState('networkidle');
  await page
    .locator('section.retention')
    .screenshot({ path: '.local/retention.png', animations: 'disabled' });
  writeFileSync(
    '.local/retention-result.json',
    JSON.stringify(
      {
        passed: true,
        keep: true,
        deletion: true,
        sameManagerEntry: before.id === after.id,
        ownedExclusionReversed: true,
        reacquisitionSearch: true,
        browser: true,
      },
      null,
      2,
    ),
  );
  console.log(
    'Generated movie retention, Keep, exclusion reversal, reacquisition and browser checks passed',
  );
} finally {
  if (original) await api('/admin/retention/policy/movies', 'POST', original);
  await browser.close();
}
