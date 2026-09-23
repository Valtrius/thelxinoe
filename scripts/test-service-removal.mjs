// Isolated real Docker/API/browser checks. Build Dockerfile's server/controller
// targets as thelxinoe-service-{server,controller}:local before running.
import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer } from 'node:net';

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
const project = `thelxinoe-removal-${Date.now()}`;
const root = resolve(`.local/${project}`);
process.env.THELXINOE_CONNECTIONS_ROOT = root;
process.env.THELXINOE_CONNECTIONS_PORT = String(await port());
for (const dir of ['server', 'cache', 'media'])
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

try {
  compose('up', '-d', '--wait', '--wait-timeout', '180');
  await api('/setup', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  deployment = (await stack()).deployment_id;
  for (const [kind, folder] of [
    ['radarr', 'movies'],
    ['sonarr', 'tv'],
    ['lidarr', 'music'],
  ]) {
    expect(existsSync(`${root}/media/${folder}`)).toBe(false);
    await install(kind);
    // No defaults save or options read precedes this check.
    expect(existsSync(`${root}/media/${folder}`)).toBe(true);
    const roots = await upstream(kind, 'rootfolder');
    expect(roots).toHaveLength(1);
    expect(roots[0].path).toBe(`/media/${folder}`);
    expect(roots[0].id).toBeGreaterThan(0);
    console.log(`${kind}: library root created during installation`);
  }
  await install('prowlarr');
  await configure('prowlarr', 'radarr', 'connect');
  await configure('prowlarr', 'sonarr', 'connect');
  await waitLink('prowlarr', 'radarr', 'connected');
  await waitLink('prowlarr', 'sonarr', 'connected');
  expect(await upstream('prowlarr', 'applications')).toHaveLength(2);
  const radarr = { ...services.radarr };
  await expect(action('radarr', 'remove')).rejects.toThrow(/Stop the service/);
  await idle('prowlarr');
  await action('prowlarr', 'stop');
  await idle('radarr');
  await action('radarr', 'stop');
  writeFileSync(
    `${root}/media/movies/keep.txt`,
    'media survives service removal',
  );
  await action('radarr', 'remove');
  expect((await stack()).items.some((item) => item.id === radarr.id)).toBe(
    false,
  );
  expect(
    (await api('/admin/managers')).items.some((item) => item.kind === 'radarr'),
  ).toBe(false);
  expect(() => docker('inspect', radarr.container_id)).toThrow();
  const remaining = compose(
    'exec',
    '-T',
    'controller',
    'ls',
    '-A',
    `/var/lib/thelxinoe/deployment/services/${radarr.id}`,
  );
  expect(remaining).toBe('service.json');
  expect(readFileSync(`${root}/media/movies/keep.txt`, 'utf8')).toBe(
    'media survives service removal',
  );
  const pending = await link('prowlarr', 'radarr');
  expect(pending.enabled).toBe(false);
  expect(pending.cleanup_pending).toBe(true);
  await action('prowlarr', 'start');
  await expect
    .poll(
      async () =>
        (await links()).some((item) => item.target_id === pending.target_id),
      { timeout: 90000, intervals: [1500] },
    )
    .toBe(false);
  const applications = await upstream('prowlarr', 'applications');
  expect(applications).toHaveLength(1);
  expect(applications[0].implementation).toBe('Sonarr');
  expect(
    (await stack()).items.find((item) => item.kind === 'sonarr').running,
  ).toBe(true);
  console.log(
    'Stopped service removed; appdata erased; incoming link cleaned after source restart; media and other services preserved',
  );
  // A new installation of the same kind must start with fresh appdata.
  await install('radarr');
  expect(services.radarr.id).not.toBe(radarr.id);
  expect((await upstream('radarr', 'rootfolder'))[0].path).toBe(
    '/media/movies',
  );
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  console.log(
    'Fresh installation after removal succeeded without reconnecting old links',
  );
  // Removal also works after an external deletion, without an API idle check.
  docker('rm', '-f', services.lidarr.container_id);
  await action('lidarr', 'remove');
  expect(
    (await stack()).provisions.some((item) => item.kind === 'lidarr'),
  ).toBe(false);
  console.log('Externally deleted service removed with its configuration');
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify({ passed: true, project, deployment }, null, 2),
  );
  passed = true;
} finally {
  await browser.close();
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
  console.log(
    `Service removal checks ${passed ? 'passed' : 'failed'}: ${root}`,
  );
}
