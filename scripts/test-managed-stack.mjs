import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const b = await chromium.launch();
const c = await b.newContext({ ignoreHTTPSErrors: true });
async function api(p, m = 'GET', data) {
  const r = await c.request.fetch('https://localhost:24443/api/v1' + p, {
    method: m,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  if (!r.ok())
    throw Error(`${p}: ${r.status()} ${(await r.json()).error?.message}`);
  return r.json();
}
try {
  const auth = { username: 'admin', password: 'test-only long passphrase' };
  if ((await api('/setup')).setup_required) {
    await api('/setup', 'POST', auth);
  } else await api('/auth/login', 'POST', auth);
  let last;
  for (const [kind, host_port] of [
    ['radarr', 37878],
    ['sonarr', 38989],
    ['lidarr', 38686],
    ['nzbget', 36789],
    ['prowlarr', 39696],
    ['bazarr', 36767],
  ]) {
    const state = await api('/admin/stack');
    if (!state.provisions.some((p) => p.kind === kind))
      await api('/admin/stack/install', 'POST', { kind, host_port });
    await expect
      .poll(
        async () => {
          last = await api('/admin/stack');
          const row = last.provisions.find((p) => p.kind === kind);
          if (row?.state === 'blocked')
            throw Error(
              JSON.stringify({
                kind: row.kind,
                state: row.state,
                error: row.error,
              }),
            );
          return row?.state;
        },
        { timeout: 240000, intervals: [2000] },
      )
      .toBe('complete');
    console.log(kind + ': installed and connected');
  }
  writeFileSync(
    '.local/managed-install-result.json',
    JSON.stringify(last, null, 2),
  );
  console.log(JSON.stringify(last));
} finally {
  await b.close();
}
