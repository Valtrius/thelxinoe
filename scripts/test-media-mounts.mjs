// Isolated real Docker proof; does not touch existing deployments or credentials.
import { request, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, existsSync, writeFileSync, readFileSync } from 'node:fs';
import { randomBytes } from 'node:crypto';

const root = '.local/media-mount-test';
if (existsSync(root))
  throw Error('Use a fresh .local/media-mount-test directory.');
for (const dir of [
  'server',
  'cache',
  'radarr',
  'media/tv',
  'media/movies',
  'media/downloads',
  'media/music',
  'media/old-movies',
]) {
  mkdirSync(`${root}/${dir}`, { recursive: true });
}
writeFileSync(`${root}/media/movies/visible.txt`, 'external movie');
writeFileSync(`${root}/media/tv/visible.txt`, 'managed TV');
const key = randomBytes(16).toString('hex');
writeFileSync(
  `${root}/radarr/config.xml`,
  `<Config><BindAddress>*</BindAddress><Port>7878</Port><EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>${key}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><UpdateAutomatically>False</UpdateAutomatically></Config>`,
);

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
const compose = (...args) =>
  docker('compose', '-f', 'compose.media.test.yaml', ...args);
const inspect = (id) => JSON.parse(docker('inspect', id))[0];
const client = await request.newContext({
  ignoreHTTPSErrors: true,
  baseURL: 'https://localhost:31443/api/v1',
});
let deployment;
async function api(path, method = 'GET', data) {
  const response = await client.fetch('https://localhost:31443/api/v1' + path, {
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
async function radarr(path, method = 'GET', data) {
  const response = await client.fetch('http://127.0.0.1:37879/api/v3/' + path, {
    method,
    data,
    headers: { 'X-Api-Key': key },
  });
  if (!response.ok()) throw Error(`Radarr ${path}: ${response.status()}`);
  const body = await response.text();
  return body ? JSON.parse(body) : null;
}
try {
  compose('up', '-d', '--wait', '--wait-timeout', '180');
  await api('/setup', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  deployment = (await api('/admin/stack')).deployment_id;
  const server = compose('ps', '-q', 'server');
  const external = compose('ps', '-q', 'radarr');
  await expect
    .poll(
      async () => {
        try {
          return (await radarr('system/status')).appName;
        } catch {
          return '';
        }
      },
      { timeout: 90000 },
    )
    .toBe('Radarr');
  const badRoot = await radarr('rootfolder', 'POST', {
    path: '/media/old-movies',
  });
  const rejected = await client.post(
    'https://localhost:31443/api/v1/admin/managers',
    {
      data: {
        name: 'External NAS Radarr',
        kind: 'radarr',
        container_id: external,
        port: 7878,
        api_key: key,
      },
      headers: { 'X-Thelxinoe-Client': '1' },
    },
  );
  expect(rejected.status()).toBe(409);
  expect((await rejected.json()).error.message).toContain('/media/movies');
  await radarr(`rootfolder/${badRoot.id}`, 'DELETE');
  await radarr('rootfolder', 'POST', { path: '/media/movies' });
  const connected = await api('/admin/managers', 'POST', {
    name: 'External NAS Radarr',
    kind: 'radarr',
    container_id: external,
    port: 7878,
    api_key: key,
  });
  const options = await api(`/admin/managers/${connected.id}/options`);
  expect(options.roots.some((r) => r.path === '/media/movies')).toBe(true);
  await api(`/admin/managers/${connected.id}/defaults`, 'PUT', {
    root_folder: '/media/movies',
    quality_profile: options.profiles[0].id,
    metadata_profile: null,
    monitored: false,
  });
  expect(docker('exec', server, 'cat', '/media/movies/visible.txt')).toBe(
    'external movie',
  );
  expect(docker('exec', external, 'cat', '/media/movies/visible.txt')).toBe(
    'external movie',
  );
  console.log(
    'External Compose-owned Radarr connected using the shared /media mount.',
  );
  await api('/admin/stack/install', 'POST', {
    kind: 'sonarr',
    host_port: 38990,
  });
  let stack;
  await expect
    .poll(
      async () => {
        stack = await api('/admin/stack');
        const provision = stack.provisions.find((p) => p.kind === 'sonarr');
        if (provision?.state === 'blocked')
          throw Error(`Sonarr: ${provision.error}`);
        return provision?.state;
      },
      { timeout: 300000, intervals: [2000] },
    )
    .toBe('complete');
  const sonarr = stack.items.find((s) => s.kind === 'sonarr');
  const raw = inspect(sonarr.container_id);
  const mounts = raw.Mounts;
  const serverMounts = inspect(server).Mounts.filter(
    (m) => m.Destination === '/media' || m.Destination.startsWith('/media/'),
  );
  for (const expected of serverMounts) {
    const actual = mounts.find((m) => m.Destination === expected.Destination);
    expect(actual.Source).toBe(expected.Source);
    expect(actual.RW).toBe(expected.RW);
  }
  expect(serverMounts).toHaveLength(1);
  expect(
    mounts.filter(
      (m) => m.Destination === '/media' || m.Destination.startsWith('/media/'),
    ),
  ).toHaveLength(1);
  expect(mounts.find((m) => m.Destination === '/media').RW).toBe(true);
  expect(
    docker('exec', sonarr.container_id, 'cat', '/media/tv/visible.txt'),
  ).toBe('managed TV');
  docker(
    'exec',
    '-u',
    '10001:10001',
    sonarr.container_id,
    'sh',
    '-c',
    'printf "sonarr write" > /media/tv/created.txt',
  );
  expect(readFileSync(`${root}/media/tv/created.txt`, 'utf8')).toBe(
    'sonarr write',
  );
  const manager = (await api('/admin/managers')).items.find(
    (m) => m.kind === 'sonarr',
  );
  const choices = await api(`/admin/managers/${manager.id}/options`);
  expect(choices.roots.some((r) => r.path === '/media/tv')).toBe(true);
  await api(`/admin/managers/${manager.id}/defaults`, 'PUT', {
    root_folder: '/media/tv',
    quality_profile: choices.profiles[0].id,
    metadata_profile: null,
    monitored: false,
  });
  const created = await api(`/admin/managers/${manager.id}/options`);
  expect(created.roots.some((r) => r.path === '/media/tv' && r.id > 0)).toBe(
    true,
  );
  console.log(
    'Managed Sonarr uses the same single /media mount; /media/tv created and writable.',
  );
  const controller = compose('ps', '-q', 'controller');
  const recovery = JSON.parse(
    docker(
      'exec',
      controller,
      'cat',
      '/var/lib/thelxinoe/deployment/compose.override.yaml',
    ),
  );
  const recoveryMedia = recovery.services.server.volumes.filter(
    (v) => v.target === '/media' || v.target.startsWith('/media/'),
  );
  expect(recoveryMedia).toHaveLength(1);
  expect(recoveryMedia[0].target).toBe('/media');
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify(
      {
        external_radarr: true,
        managed_sonarr: true,
        single_shared_mount: true,
        noncanonical_root_rejected: true,
        tv_root_created: true,
        recovery_mounts: true,
      },
      null,
      2,
    ),
  );
  console.log('Media mount validation passed.');
} finally {
  if (deployment) {
    const containers = docker(
      'ps',
      '-aq',
      '--filter',
      `label=app.thelxinoe.deployment=${deployment}`,
    )
      .split(/\s+/)
      .filter(Boolean);
    if (containers.length) docker('rm', '-f', ...containers);
  }
  compose('down', '--volumes', '--remove-orphans');
  await client.dispose();
}
