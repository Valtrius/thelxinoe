// Fault injection is restricted to the labeled disposable managed-v3 fixture.
import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
const docker = (args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'pipe'],
  }).trim();
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(
    'https://localhost:24443/api/v1' + path,
    { method, data, headers: { 'X-Thelxinoe-Client': '1' } },
  );
  expect(r.status(), path).toBe(200);
  return r.json();
}
async function waitUpdate(id, states) {
  let found;
  await expect
    .poll(
      async () => {
        found = (await api('/admin/service-updates')).items.find(
          (u) => u.id === id,
        );
        return states.includes(found?.state);
      },
      { timeout: 300000, intervals: [1000] },
    )
    .toBe(true);
  return found;
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const stack = await api('/admin/stack');
  const service = stack.items.find((s) => s.kind === 'radarr');
  const raw = JSON.parse(docker(['inspect', service.container_id]))[0];
  expect(raw.Config.Labels['app.thelxinoe.deployment']).toBe(
    stack.deployment_id,
  );
  expect(Object.keys(raw.NetworkSettings.Networks)).toEqual([
    'thelxinoe-managed-v3_test',
  ]);
  const verified = JSON.parse(
    readFileSync('.local/service-preflights.json'),
  ).find((u) => u.kind === 'radarr' && u.state === 'ready');
  expect(verified).toBeTruthy();
  const original = service.container_id;
  docker([
    'exec',
    original,
    'sh',
    '-c',
    'printf before-update > /config/update-sentinel',
  ]);
  await api('/admin/service-updates/' + verified.id + '/activate', 'POST', {});
  let candidate;
  await expect
    .poll(
      () => {
        const ids = docker([
          'ps',
          '--filter',
          'label=app.thelxinoe.update=' + verified.id,
          '--filter',
          'name=thelxinoe-candidate',
          '--format',
          '{{.ID}}',
        ]);
        candidate = ids.split('\n').find(Boolean);
        return !!candidate;
      },
      { timeout: 60000, intervals: [100] },
    )
    .toBe(true);
  const isolated = JSON.parse(docker(['inspect', candidate]))[0];
  expect(isolated.HostConfig.NetworkMode).toBe('none');
  expect(isolated.HostConfig.PortBindings ?? {}).toEqual({});
  expect(
    isolated.Mounts.some(
      (m) =>
        m.Destination.includes('docker.sock') ||
        m.Destination.includes('thelxinoe/controller'),
    ),
  ).toBe(false);
  expect(isolated.Mounts.find((m) => m.Destination === '/config').Source).toBe(
    raw.Mounts.find((m) => m.Destination === '/config').Source,
  );
  // Simulate a migration writing appdata, then a candidate process failing.
  docker([
    'exec',
    candidate,
    'sh',
    '-c',
    'printf candidate-mutation > /config/rollback-probe; printf changed > /config/update-sentinel',
  ]);
  docker(['stop', '-t', '1', candidate]);
  const rolled = await waitUpdate(verified.id, [
    'rolled-back',
    'recovery-required',
    'runtime-failure',
  ]);
  expect(rolled.state).toBe('rolled-back');
  expect(docker(['exec', original, 'cat', '/config/update-sentinel'])).toBe(
    'before-update',
  );
  expect(
    docker([
      'exec',
      original,
      'sh',
      '-c',
      'test ! -e /config/rollback-probe && printf absent',
    ]),
  ).toBe('absent');
  const after = (await api('/admin/stack')).items.find(
    (s) => s.id === service.id,
  );
  expect(after.container_id).toBe(original);
  expect(after.drift).toBe(false);
  expect(after.running).toBe(true);
  const integration = stack.provisions.find(
    (p) => p.id === service.id,
  ).service_id;
  await expect
    .poll(
      async () => {
        try {
          return (
            await api('/admin/managers/' + integration + '/options')
          ).roots.some((r) => r.path === '/data/movies');
        } catch {
          return false;
        }
      },
      { timeout: 60000, intervals: [2000] },
    )
    .toBe(true);
  const result = {
    pre_activation_rollback: true,
    appdata_restored: true,
    candidate_only_loopback: true,
    original_container_restored: true,
  };
  writeFileSync(
    '.local/service-rollback-result.json',
    JSON.stringify(result, null, 2),
  );
  console.log(result);
} finally {
  await browser.close();
}
