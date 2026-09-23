// Isolated real Docker/API/browser checks. Build Dockerfile's server/controller
// targets as thelxinoe-service-{server,controller}:local before running.
import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer } from 'node:net';
import { randomUUID } from 'node:crypto';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
async function port() {
  const server = createServer();
  await new Promise((done, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', done);
  });
  const result = server.address().port;
  await new Promise((done) => server.close(done));
  return result;
}
const project = `thelxinoe-connections-${Date.now()}`;
const root = resolve(`.local/${project}`);
process.env.THELXINOE_CONNECTIONS_ROOT = root;
process.env.THELXINOE_CONNECTIONS_PORT = String(await port());
for (const dir of [
  'server',
  'cache',
  'media/movies',
  'media/tv',
  'media/music',
  'media/downloads',
])
  mkdirSync(`${root}/${dir}`, { recursive: true });
const compose = (...args) =>
  docker(
    'compose',
    '-p',
    project,
    '-f',
    'compose.connections.test.yaml',
    ...args,
  );
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const base = `https://localhost:${process.env.THELXINOE_CONNECTIONS_PORT}`;
const services = {};
let deployment,
  passed = false;
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(`${base}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
  if (!response.ok())
    throw Error(
      `${path}: ${response.status()} ${(await response.json()).error?.message}`,
    );
  return response.json();
}
const stack = () => api('/admin/stack');
const links = async () => (await api('/admin/service-connections')).items;
const link = async (source, target) =>
  (await links()).find(
    (l) => l.source_kind === source && l.target_kind === target,
  );
async function configure(source, target, action) {
  const item = await link(source, target);
  return api('/admin/service-connections', 'POST', {
    source_id: item.source_id,
    target_id: item.target_id,
    action,
  });
}
async function waitLink(source, target, expected) {
  let latest;
  try {
    await expect
      .poll(
        async () => {
          latest = await link(source, target);
          return latest?.state;
        },
        { timeout: 180000, intervals: [1500] },
      )
      .toBe(expected);
  } catch {
    throw Error(`${source} -> ${target}: ${latest?.state}: ${latest?.error}`);
  }
}
const action = (kind, name) =>
  api(`/admin/stack/${services[kind].id}/action`, 'POST', { action: name });
async function install(kind) {
  const host_port = await port();
  const created = await api('/admin/stack/install', 'POST', {
    kind,
    host_port,
  });
  let latest;
  await expect
    .poll(
      async () => {
        latest = await stack();
        const provision = latest.provisions.find((p) => p.id === created.id);
        if (provision?.state === 'blocked')
          throw Error(`${kind}: ${provision.error}`);
        return provision?.state;
      },
      { timeout: 300000, intervals: [2000] },
    )
    .toBe('complete');
  services[kind] = {
    ...latest.items.find((s) => s.id === created.id),
    host_port,
  };
  console.log(`${kind}: installed independently`);
}
function config(kind) {
  const filename =
    kind === 'bazarr'
      ? 'config/config.yaml'
      : kind === 'nzbget'
        ? 'nzbget.conf'
        : 'config.xml';
  return compose(
    'exec',
    '-T',
    '-u',
    '10001:10001',
    'controller',
    'cat',
    `/var/lib/thelxinoe/deployment/services/${services[kind].id}/appdata/${filename}`,
  );
}
async function upstream(kind, path, method = 'GET', data) {
  const raw = config(kind);
  const key =
    kind === 'bazarr'
      ? raw.match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1]
      : raw.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const prefix =
    kind === 'bazarr'
      ? 'api'
      : `api/v${['lidarr', 'prowlarr'].includes(kind) ? 1 : 3}`;
  const response = await context.request.fetch(
    `http://localhost:${services[kind].host_port}/${prefix}/${path}`,
    {
      method,
      data,
      headers: { 'X-Api-Key': key },
    },
  );
  if (!response.ok()) throw Error(`${kind}/${path}: HTTP ${response.status()}`);
  return response.status() === 204 ||
    response.headers()['content-length'] === '0'
    ? null
    : response.json();
}
async function nzbget(method, params = []) {
  const raw = config('nzbget');
  const username = raw.match(/^ControlUsername=(.+)$/m)[1].trim();
  const password = raw.match(/^ControlPassword=(.+)$/m)[1].trim();
  const response = await context.request.post(
    `http://localhost:${services.nzbget.host_port}/jsonrpc`,
    {
      headers: {
        Authorization: `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`,
      },
      data: { method, params, id: 1 },
    },
  );
  const result = await response.json();
  expect(result.error).toBeFalsy();
  return result.result;
}
async function idle(kind) {
  await expect
    .poll(
      async () =>
        (await upstream(kind, 'command')).filter((c) =>
          ['queued', 'started'].includes(c.status),
        ).length,
      { timeout: 120000, intervals: [2000] },
    )
    .toBe(0);
}
async function refreshService(kind) {
  const latest = await stack();
  Object.assign(
    services[kind],
    latest.items.find((s) => s.id === services[kind].id),
  );
}
try {
  compose('up', '-d', '--wait', '--wait-timeout', '180');
  await api('/setup', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  deployment = (await stack()).deployment_id;
  await install('prowlarr');
  await idle('prowlarr');
  await action('prowlarr', 'stop');
  await install('radarr');
  expect((await link('prowlarr', 'radarr')).state).toBe('available');
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  const page = await context.newPage();
  await page.goto(`${base}/?section=Settings`);
  await page
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  await page
    .getByRole('article', { name: 'Prowlarr service' })
    .locator('summary')
    .first()
    .click();
  const applications = page.getByRole('region', {
    name: 'Applications',
    exact: true,
  });
  await expect(applications.getByText('Available to connect')).toBeVisible();
  await applications
    .getByRole('button', { name: 'Connect', exact: true })
    .click();
  await waitLink('prowlarr', 'radarr', 'unavailable');
  await expect(
    applications.getByText('Connection unavailable', { exact: true }),
  ).toBeVisible({ timeout: 15000 });
  await page.screenshot({
    path: `${root}/optional-connection-outage.png`,
    fullPage: true,
  });
  compose('restart', 'server');
  await expect
    .poll(
      async () => {
        try {
          return (await api('/health')).status;
        } catch {
          return '';
        }
      },
      { timeout: 60000 },
    )
    .toBe('ok');
  expect((await link('prowlarr', 'radarr')).enabled).toBe(true);
  await action('prowlarr', 'start');
  await waitLink('prowlarr', 'radarr', 'connected');
  let records = await upstream('prowlarr', 'applications');
  expect(records).toHaveLength(1);
  const original = records[0];
  original.name = 'My renamed movie manager';
  await upstream('prowlarr', `applications/${original.id}`, 'PUT', original);
  await configure('prowlarr', 'radarr', 'retry');
  await waitLink('prowlarr', 'radarr', 'connected');
  expect((await upstream('prowlarr', 'applications'))[0].name).toBe(
    original.name,
  );
  await idle('prowlarr');
  await action('prowlarr', 'stop');
  await configure('prowlarr', 'radarr', 'disconnect');
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  await idle('radarr');
  await action('radarr', 'stop');
  await action('prowlarr', 'start');
  await waitLink('prowlarr', 'radarr', 'disconnected');
  expect(await upstream('prowlarr', 'applications')).toHaveLength(0);
  await action('radarr', 'start');
  console.log(
    'Optional connection survives restart; disconnect cleans up with target stopped',
  );

  for (const kind of ['sonarr', 'lidarr', 'nzbget', 'bazarr'])
    await install(kind);
  const customCategory = await nzbget('loadconfig');
  customCategory.push(
    { Name: 'Category1.Name', Value: 'My custom downloads' },
    { Name: 'Category1.DestDir', Value: '/media/downloads/custom' },
  );
  expect(await nzbget('saveconfig', [customCategory])).toBe(true);
  await nzbget('reload');
  await expect
    .poll(
      async () => {
        try {
          return (await nzbget('config')).some(
            (r) =>
              r.Name === 'Category1.Name' && r.Value === 'My custom downloads',
          );
        } catch {
          return false;
        }
      },
      { timeout: 60000 },
    )
    .toBe(true);
  expect(await links()).toHaveLength(8);
  expect((await links()).every((l) => !l.enabled)).toBe(true);
  for (const item of await links())
    await configure(item.source_kind, item.target_kind, 'connect');
  for (const item of await links())
    await waitLink(item.source_kind, item.target_kind, 'connected');
  expect(await upstream('prowlarr', 'applications')).toHaveLength(3);
  for (const kind of ['radarr', 'sonarr', 'lidarr'])
    expect(await upstream(kind, 'downloadclient')).toHaveLength(1);
  const subtitles = await upstream('bazarr', 'system/settings');
  expect(subtitles.general.use_radarr).toBe(true);
  expect(subtitles.general.use_sonarr).toBe(true);
  const savedCategory = await nzbget('loadconfig');
  expect(savedCategory.find((r) => r.Name === 'Category1.Name').Value).toBe(
    'My custom downloads',
  );
  expect(savedCategory.find((r) => r.Name === 'Category1.DestDir').Value).toBe(
    '/media/downloads/custom',
  );
  console.log('All eight explicit connections verified against real APIs');

  const changed = (await upstream('prowlarr', 'applications')).find(
    (r) => r.implementation === 'Radarr',
  );
  changed.syncLevel = 'addOnly';
  await upstream('prowlarr', `applications/${changed.id}`, 'PUT', changed);
  await configure('prowlarr', 'radarr', 'retry');
  await waitLink('prowlarr', 'radarr', 'conflict');
  expect(
    (await upstream('prowlarr', 'applications')).find(
      (r) => r.id === changed.id,
    ).syncLevel,
  ).toBe('addOnly');
  await configure('prowlarr', 'radarr', 'disconnect');
  await waitLink('prowlarr', 'radarr', 'conflict');
  expect(
    (await upstream('prowlarr', 'applications')).some(
      (r) => r.id === changed.id,
    ),
  ).toBe(true);
  await upstream('prowlarr', `applications/${changed.id}`, 'DELETE');
  await configure('prowlarr', 'radarr', 'retry');
  await waitLink('prowlarr', 'radarr', 'disconnected');
  await configure('prowlarr', 'radarr', 'connect');
  await waitLink('prowlarr', 'radarr', 'connected');
  console.log(
    'Manual connection edits and custom NZBGet category slots are preserved',
  );

  // Real missing-container recovery must preserve config and integration identity.
  const before = services.radarr.container_id;
  const identity = (await api('/admin/managers')).items.find(
    (m) => m.kind === 'radarr',
  ).id;
  const sentinel = randomUUID();
  const appdata = `/var/lib/thelxinoe/deployment/services/${services.radarr.id}/appdata`;
  compose(
    'exec',
    '-T',
    '-u',
    '10001:10001',
    'controller',
    'sh',
    '-c',
    `printf '%s' '${sentinel}' > '${appdata}/recovery-sentinel'`,
  );
  await idle('radarr');
  await action('radarr', 'stop');
  docker('rm', before);
  await refreshService('radarr');
  expect(services.radarr.status).toBe('missing');
  expect(services.radarr.drift).toBe(null);
  await action('radarr', 'recreate');
  await expect
    .poll(
      async () =>
        (await stack()).provisions.find((p) => p.id === services.radarr.id)
          ?.state,
      { timeout: 120000, intervals: [2000] },
    )
    .toBe('complete');
  await refreshService('radarr');
  expect(services.radarr.container_id).not.toBe(before);
  const manager = (await api('/admin/managers')).items.find(
    (m) => m.kind === 'radarr',
  );
  expect(manager.id).toBe(identity);
  expect(manager.container_id).toBe(services.radarr.container_id);
  expect(
    compose(
      'exec',
      '-T',
      '-u',
      '10001:10001',
      'controller',
      'cat',
      `${appdata}/recovery-sentinel`,
    ),
  ).toBe(sentinel);
  await configure('prowlarr', 'radarr', 'retry');
  await waitLink('prowlarr', 'radarr', 'connected');
  expect(await upstream('prowlarr', 'applications')).toHaveLength(3);
  console.log(
    'Missing container recreated with preserved config, integration and connection',
  );

  // A creating journal with a lost create response must accept exactly one
  // owned container. With no container it must recreate the same saved spec.
  const journalPath = `/var/lib/thelxinoe/deployment/services/${services.radarr.id}/service.json`;
  for (const removeContainer of [false, true]) {
    await refreshService('radarr');
    const previous = services.radarr.container_id;
    if (removeContainer) {
      await idle('radarr');
      await action('radarr', 'stop');
      docker('rm', previous);
    }
    const journal = JSON.parse(
      compose('exec', '-T', 'controller', 'cat', journalPath),
    );
    journal.phase = 'creating';
    journal.container = '';
    journal.expected = null;
    writeFileSync(`${root}/interrupted-service.json`, JSON.stringify(journal));
    docker(
      'cp',
      `${root}/interrupted-service.json`,
      `${compose('ps', '-q', 'controller')}:${journalPath}`,
    );
    await action('radarr', 'reconcile');
    await expect
      .poll(
        async () =>
          (await stack()).provisions.find((p) => p.id === services.radarr.id)
            ?.state,
        { timeout: 120000, intervals: [2000] },
      )
      .toBe('complete');
    await refreshService('radarr');
    expect(services.radarr.container_id === previous).toBe(!removeContainer);
    expect(
      docker(
        'ps',
        '-aq',
        '--filter',
        `label=app.thelxinoe.managed-id=${services.radarr.id}`,
      ).split(/\s+/),
    ).toHaveLength(1);
  }
  console.log(
    'Interrupted creation recovers before and after the Docker create response',
  );

  await idle('radarr');
  await action('radarr', 'stop');
  await action('radarr', 'stop');
  await action('radarr', 'restart');
  await expect
    .poll(
      async () => {
        try {
          return (await upstream('radarr', 'system/status')).appName;
        } catch {
          return '';
        }
      },
      { timeout: 90000 },
    )
    .toBe('Radarr');
  await idle('radarr');
  await action('radarr', 'stop');
  let update = await api(
    `/admin/service-updates/preflight/${services.radarr.id}`,
    'POST',
    {},
  );
  async function waitUpdate(stage) {
    let latest;
    await expect
      .poll(
        async () => {
          latest = (await api('/admin/service-updates')).items.find(
            (item) => item.id === update.id,
          );
          if (
            [
              'blocked',
              'runtime-failure',
              'recovery-required',
              'rolled-back',
            ].includes(latest?.state)
          )
            throw Error(`Update ${latest.state}: ${latest.error}`);
          return latest?.state;
        },
        { timeout: 360000, intervals: [2000] },
      )
      .toBe(stage);
  }
  await waitUpdate('ready');
  await refreshService('radarr');
  expect(services.radarr.running).toBe(false);
  const preUpdate = services.radarr.container_id;
  await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
  await waitUpdate('committed');
  await refreshService('radarr');
  expect(services.radarr.container_id).not.toBe(preUpdate);
  expect(services.radarr.running).toBe(false);
  expect(
    (await api('/admin/managers')).items.find((m) => m.id === identity)
      .container_id,
  ).toBe(services.radarr.container_id);
  console.log(
    'Stopped service completes isolated preflight and activation, remains stopped',
  );

  await action('radarr', 'start');
  await expect
    .poll(
      async () => {
        try {
          return (await upstream('radarr', 'system/status')).appName;
        } catch {
          return '';
        }
      },
      { timeout: 90000 },
    )
    .toBe('Radarr');
  await idle('radarr');
  update = await api(
    `/admin/service-updates/preflight/${services.radarr.id}`,
    'POST',
    {},
  );
  await waitUpdate('ready');
  await refreshService('radarr');
  expect(services.radarr.running).toBe(true);
  await idle('radarr');
  await action('radarr', 'stop');
  await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
  await waitUpdate('committed');
  await refreshService('radarr');
  expect(services.radarr.running).toBe(false);
  console.log(
    'Activation also respects a deliberate stop made after preflight',
  );

  // Retained stopped update originals must not be mistaken for the current
  // missing container or prevent explicit retirement and reinstall.
  docker('rm', services.radarr.container_id);
  await action('radarr', 'recreate');
  await expect
    .poll(
      async () =>
        (await stack()).provisions.find((p) => p.id === services.radarr.id)
          ?.state,
      { timeout: 120000, intervals: [2000] },
    )
    .toBe('complete');
  await refreshService('radarr');
  await idle('radarr');
  await action('radarr', 'stop');
  docker('rm', services.radarr.container_id);
  await action('radarr', 'retire');
  expect(
    (await api('/admin/managers')).items.some((m) => m.kind === 'radarr'),
  ).toBe(false);
  expect(
    compose(
      'exec',
      '-T',
      '-u',
      '10001:10001',
      'controller',
      'cat',
      `${appdata}/recovery-sentinel`,
    ),
  ).toBe(sentinel);
  await install('radarr');
  expect(
    (await api('/admin/managers')).items.find((m) => m.kind === 'radarr').id,
  ).toBe(identity);
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  console.log(
    'Recovery after update and retirement preserve appdata; reinstall releases the old reservation',
  );
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify(
      {
        project,
        passed: true,
        connections: (await links()).map((l) => ({
          source: l.source_kind,
          target: l.target_kind,
          state: l.state,
        })),
      },
      null,
      2,
    ),
  );
  passed = true;
} finally {
  await browser.close();
  if (passed || process.env.THELXINOE_KEEP_FAILED_FIXTURE !== '1') {
    if (deployment) {
      const owned = docker(
        'ps',
        '-aq',
        '--filter',
        `label=app.thelxinoe.deployment=${deployment}`,
      )
        .split(/\s+/)
        .filter(Boolean);
      if (owned.length) docker('rm', '-f', ...owned);
    }
    compose('down', '-v');
  } else {
    writeFileSync(
      `${root}/fixture.json`,
      JSON.stringify({ project, root, deployment, base, services }, null, 2),
    );
    console.log(`Failed fixture retained: ${root}`);
  }
}
