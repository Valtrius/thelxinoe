// Isolated adoption proof: only the named adoption fixture is created or replaced.
import { chromium, expect } from '@playwright/test';
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomBytes, randomUUID } from 'node:crypto';
const compose = ['compose', '-f', 'compose.adoption.test.yaml'];
const docker = (args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'pipe'],
  }).trim();
const image =
  'lscr.io/linuxserver/radarr@sha256:c960f2b52ec6542dbe6707c5a21e696a7c74fd8b17997454f4d10a55dacee133';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(
    'https://localhost:25443/api/v1' + path,
    { method, data, headers: { 'X-Thelxinoe-Client': '1' } },
  );
  if (!r.ok())
    throw Error(
      `${path}: HTTP ${r.status()} ${(await r.json()).error?.message}`,
    );
  return r.json();
}
try {
  const auth = { username: 'admin', password: 'test-only long passphrase' };
  if ((await api('/setup')).setup_required) {
    await api('/setup', 'POST', auth);
  } else await api('/auth/login', 'POST', auth);
  const state = await api('/admin/stack');
  const foreign = docker([
    'create',
    '--label',
    'com.docker.compose.project=foreign-adoption-fixture',
    image,
  ]);
  try {
    const result = docker([
      ...compose,
      'exec',
      '-T',
      'controller',
      'curl',
      '-sS',
      '--unix-socket',
      '/run/thelxinoe/controller.sock',
      '-o',
      '/dev/null',
      '-w',
      '%{http_code}',
      '-H',
      'Content-Type: application/json',
      '-d',
      JSON.stringify({
        operation_id: randomUUID(),
        kind: 'radarr',
        container_id: foreign,
      }),
      'http://controller/stack/adopt',
    ]);
    expect(result).toBe('409');
  } finally {
    docker(['rm', foreign]);
  }
  if (state.provisions.some((p) => p.kind === 'radarr'))
    throw Error(
      'Adoption fixture already used; inspect the saved result instead of recreating it.',
    );
  const directory = resolve('.local/adoption/radarr');
  mkdirSync(directory, { recursive: true });
  const path = directory + '/config.xml';
  if (!existsSync(path))
    writeFileSync(
      path,
      `<Config><BindAddress>*</BindAddress><Port>7878</Port><ApiKey>${randomBytes(16).toString('hex')}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><LaunchBrowser>False</LaunchBrowser><UpdateAutomatically>False</UpdateAutomatically></Config>`,
    );
  const key = readFileSync(path, 'utf8').match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const sentinel = randomUUID();
  writeFileSync(directory + '/adoption-sentinel', sentinel);
  const original = docker([
    'run',
    '-d',
    '--name',
    'thelxinoe-adoption-radarr',
    '--network',
    'thelxinoe-adoption_test',
    '-p',
    '127.0.0.1:47878:7878',
    '-e',
    'PUID=10001',
    '-e',
    'PGID=10001',
    '-e',
    'TZ=UTC',
    '--mount',
    `type=bind,source=${directory},target=/config`,
    '--mount',
    `type=bind,source=${resolve('.local/adoption/data')},target=/media`,
    image,
  ]);
  await expect
    .poll(
      async () => {
        try {
          return (
            await fetch('http://localhost:47878/api/v3/system/status', {
              headers: { 'X-Api-Key': key },
            })
          ).ok;
        } catch {
          return false;
        }
      },
      { timeout: 90000 },
    )
    .toBe(true);
  const integration = await api('/admin/managers', 'POST', {
    name: 'Standalone fixture',
    kind: 'radarr',
    container_id: original,
    port: 7878,
    api_key: key,
  });
  await expect
    .poll(
      async () => {
        const r = await (
          await fetch('http://localhost:47878/api/v3/command', {
            headers: { 'X-Api-Key': key },
          })
        ).json();
        return r.filter((c) => ['queued', 'started'].includes(c.status)).length;
      },
      { timeout: 60000 },
    )
    .toBe(0);
  const provision = await api('/admin/stack/adopt', 'POST', {
    service_id: integration.id,
  });
  let final;
  await expect
    .poll(
      async () => {
        final = await api('/admin/stack');
        return final.provisions.find((p) => p.id === provision.id)?.state;
      },
      { timeout: 180000, intervals: [2000] },
    )
    .toBe('complete');
  const adopted = final.items.find((s) => s.id === provision.id);
  expect(adopted.drift).toBe(false);
  expect(adopted.container_id).not.toBe(original);
  expect(final.provisions.find((p) => p.id === provision.id).service_id).toBe(
    integration.id,
  );
  expect(readFileSync(directory + '/adoption-sentinel', 'utf8')).toBe(sentinel);
  expect(
    spawnSync('docker', ['inspect', original], { stdio: 'pipe' }).status,
  ).not.toBe(0);
  const result = {
    foreign_owner_rejected: true,
    integration_identity_preserved: true,
    appdata_preserved: true,
    original_retired: true,
    managed_container: adopted.container_id,
  };
  writeFileSync('.local/adoption-result.json', JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} finally {
  await browser.close();
}
