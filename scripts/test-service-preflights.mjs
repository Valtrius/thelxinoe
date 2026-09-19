import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const b = await chromium.launch();
const c = await b.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const r = await c.request.fetch('https://localhost:24443/api/v1' + path, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  if (!r.ok()) throw Error(path + ': ' + r.status() + ' ' + (await r.text()));
  return r.json();
}
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  const kind = process.argv[2];
  const services = (await api('/admin/stack')).items.filter(
    (s) => !kind || s.kind === kind,
  );
  expect(services.length).toBeGreaterThan(0);
  const result = kind
    ? JSON.parse(readFileSync('.local/service-preflights.json')).filter(
        (s) => s.kind !== kind,
      )
    : [];
  for (const s of services) {
    const u = await api('/admin/service-updates/preflight/' + s.id, 'POST', {});
    let state;
    await expect
      .poll(
        async () => {
          state = (await api('/admin/service-updates')).items.find(
            (v) => v.id === u.id,
          );
          return ['ready', 'blocked', 'recovery-required'].includes(
            state.state,
          );
        },
        { timeout: 300000, intervals: [2000] },
      )
      .toBe(true);
    result.push({ kind: s.kind, ...state });
    console.log(s.kind, state.state, state.error);
    writeFileSync(
      '.local/service-preflights.json',
      JSON.stringify(result, null, 2),
    );
  }
  expect(
    result
      .filter((s) => !kind || s.kind === kind)
      .every((s) => s.state === 'ready'),
  ).toBe(true);
} finally {
  await b.close();
}
