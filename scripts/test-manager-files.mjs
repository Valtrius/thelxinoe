// Destructive fixture test, restricted to the isolated acquisition deployment.
import { request, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
const api = await request.newContext({
  baseURL: 'https://localhost:23443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const r = await api.fetch(`/api/v1${path}`, { method, data });
  const body = await r.json();
  if (!r.ok())
    throw Error(`${path}: HTTP ${r.status()} ${body.error?.message}`);
  return body;
}
function compose(...args) {
  execFileSync(
    'docker',
    ['compose', '-f', 'compose.acquisition.test.yaml', ...args],
    { stdio: 'pipe', windowsHide: true },
  );
}
try {
  await call('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const roots = (await call('/catalog/roots')).items;
  const root =
    roots.find((r) => r.path === '/data/movies') ??
    (await call('/catalog/roots', 'POST', {
      kind: 'movies',
      name: 'Acquisition movies',
      path: '/data/movies',
    }));
  const job = (await call(`/catalog/roots/${root.id}/scan`, 'POST')).job_id;
  await expect
    .poll(
      async () =>
        (await call('/admin/jobs')).items.find((j) => j.id === job)?.state,
      { timeout: 60000 },
    )
    .toBe('complete');
  const movie = (await call('/catalog?kind=movie')).items.find(
    (m) => m.available && m.title.toLowerCase().includes('matrix'),
  );
  if (!movie)
    throw Error(
      'Import the generated Matrix fixture into Radarr before running this test',
    );
  await call('/admin/managers/reconcile', 'POST');
  const before = (await call('/admin/managers/bindings')).items;
  expect(before).toHaveLength(1);
  expect(before[0].ownership).toBe('managed');
  const prepared = await call('/admin/media/operations', 'POST', {
    media_id: movie.id,
    action: 'delete',
  });
  compose('stop', 'radarr');
  try {
    await call('/admin/managers/reconcile', 'POST');
    const outage = (await call('/admin/managers/bindings')).items[0];
    expect(outage.ownership).toBe('unresolved');
    expect(outage.bindings).toHaveLength(1);
    const denied = await api.post(
      `/api/v1/admin/media/operations/${prepared.id}/execute`,
    );
    expect(denied.status()).toBe(409);
  } finally {
    compose('start', 'radarr');
  }
  await expect
    .poll(
      async () => {
        try {
          await call('/admin/managers/reconcile', 'POST');
          return (await call('/admin/managers/bindings')).items[0].ownership;
        } catch {
          return 'waiting';
        }
      },
      { timeout: 90000, intervals: [1000, 3000] },
    )
    .toBe('managed');
  await call(`/admin/media/${movie.id}/keep`, 'PUT', { keep: true });
  expect(
    (
      await api.post(`/api/v1/admin/media/operations/${prepared.id}/execute`)
    ).status(),
  ).toBe(409);
  await call(`/admin/media/${movie.id}/keep`, 'PUT', { keep: false });
  const deleted = await call(
    `/admin/media/operations/${prepared.id}/execute`,
    'POST',
  );
  expect(deleted.state).toBe('complete');
  const after = (await call('/admin/managers/bindings')).items;
  expect(after).toHaveLength(0);
  expect(
    (
      await api.post(`/api/v1/admin/media/operations/${prepared.id}/execute`)
    ).status(),
  ).toBe(409);
  const result = {
    manager_bound: true,
    outage_blocked: true,
    history_preserved: true,
    keep_blocked: true,
    manager_delete: true,
    replay_blocked: true,
  };
  writeFileSync(
    '.local/manager-files-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await api.dispose();
}
