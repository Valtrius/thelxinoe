// Isolated real Docker/API/browser checks. Build Dockerfile's server/controller
// targets as thelxinoe-service-{server,controller}:local before running.
import { expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { composeFixture, fixtureId, freePort } from './ci-resources.mjs';
import { randomUUID } from 'node:crypto';
import { waitForProxy } from './service-access-fixture.mjs';
import { launchBrowser } from './ci-browser.mjs';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
const project = fixtureId('connections');
const root = resolve(`.local/${project}`);
process.env.THELXINOE_CONNECTIONS_ROOT = root;
process.env.THELXINOE_CONNECTIONS_PORT = String(await freePort());
for (const dir of [
  'server',
  'cache',
  'media/movies',
  'media/tv',
  'media/music',
  'media/downloads',
])
  mkdirSync(`${root}/${dir}`, { recursive: true });
let infrastructure;
const compose = (...args) => infrastructure.compose(...args);
let browser, context;
const base = `https://localhost:${process.env.THELXINOE_CONNECTIONS_PORT}`;
const services = {};
const retirementReconnects = [];
const bootstrapProfiles = {};
let deployment;
async function api(path, method = 'GET', data, timeout = 30000) {
  const response = await context.request.fetch(`${base}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout,
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
  api(
    `/admin/stack/${services[kind].id}/action`,
    'POST',
    { action: name },
    180000,
  );
async function install(kind) {
  const host_port = await freePort();
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
        : kind === 'seerr'
          ? 'settings.json'
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
      : kind === 'seerr'
        ? JSON.parse(raw).main.apiKey
        : raw.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const prefix =
    kind === 'bazarr'
      ? 'api'
      : `api/v${['lidarr', 'prowlarr', 'seerr'].includes(kind) ? 1 : 3}`;
  const urlBase =
    kind === 'bazarr'
      ? (
          raw.match(
            /^general:\r?\n(?:[ \t].*\r?\n)*?[ \t]+base_url:\s*([^\r\n]+)/m,
          )?.[1] ?? ''
        )
          .trim()
          .replace(/^['"]|['"]$/g, '')
          .replace(/\/$/, '')
      : (raw.match(/<UrlBase>(.*?)<\/UrlBase>/)?.[1] ?? '');
  const response = await context.request.fetch(
    `http://localhost:${services[kind].host_port}${urlBase}/${prefix}/${path}`,
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
async function verifyNativeProfiles(kind) {
  const manager = (await api('/admin/managers')).items.find(
    (item) => item.kind === kind,
  );
  const before = await upstream(kind, 'qualityprofile');
  expect(before.some((profile) => profile.name.startsWith('Thelxinoe'))).toBe(
    false,
  );
  let options;
  for (let read = 0; read < 2; read++)
    options = await api(`/admin/managers/${manager.id}/options`);
  expect(await upstream(kind, 'qualityprofile')).toEqual(before);
  expect(before.map((profile) => profile.id)).toContain(
    options.defaults.quality_profile,
  );
  bootstrapProfiles[kind] = {
    profiles: before.map(({ id, name }) => ({ id, name })),
    defaults: options.defaults,
  };
  console.log(`${kind}: installation and options reuse native profiles`);
}
async function saveSubtitles(form) {
  const key = config('bazarr').match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
  const response = await context.request.post(
    `${base}/services/bazarr/api/system/settings`,
    { headers: { Origin: base, 'X-Api-Key': key }, form },
  );
  expect(response.status()).toBe(204);
}
// Retirement must preserve a surviving Bazarr's ownership evidence, update
// replacement credentials on explicit reconnect, and reject manual changes.
async function reconnectSubtitles(kind, checkManualEdit = false) {
  await waitLink('bazarr', kind, 'disconnected');
  const before = await upstream('bazarr', 'system/settings');
  expect(before.general[`use_${kind}`]).toBe(false);
  expect(Boolean(before[kind].apikey)).toBe(true);
  if (checkManualEdit) {
    const customBase = `/manually-edited-${kind}`;
    await saveSubtitles({ [`settings-${kind}-base_url`]: customBase });
    await configure('bazarr', kind, 'connect');
    await waitLink('bazarr', kind, 'conflict');
    const unchanged = await upstream('bazarr', 'system/settings');
    expect(unchanged[kind].base_url.replace(/\/$/, '')).toBe(customBase);
    expect(unchanged.general[`use_${kind}`]).toBe(false);
    expect(unchanged[kind].apikey === before[kind].apikey).toBe(true);
    await saveSubtitles({
      [`settings-${kind}-base_url`]: before[kind].base_url,
    });
    await configure('bazarr', kind, 'retry');
  } else await configure('bazarr', kind, 'connect');
  await waitLink('bazarr', kind, 'connected');
  const saved = await upstream('bazarr', 'system/settings');
  expect(saved.general[`use_${kind}`]).toBe(true);
  expect(saved[kind].base_url.replace(/\/$/, '')).toBe(`/services/${kind}`);
  expect(
    saved[kind].apikey === config(kind).match(/<ApiKey>(.*?)<\/ApiKey>/)[1],
  ).toBe(true);
  retirementReconnects.push({ kind, manual_edit_checked: checkManualEdit });
  console.log(`bazarr -> ${kind}: reconnect after retirement verified`);
}
async function idle(kind) {
  await expect
    .poll(
      async () =>
        kind === 'bazarr'
          ? (await upstream(kind, 'system/tasks')).data.filter(
              (job) => job.job_running !== false,
            ).length
          : (await upstream(kind, 'command')).filter((c) =>
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
async function waitUpdate(updateId, stage) {
  let latest;
  await expect
    .poll(
      async () => {
        latest = (await api('/admin/service-updates')).items.find(
          (item) => item.id === updateId,
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

try {
  browser = await launchBrowser();
  context = await browser.newContext({ ignoreHTTPSErrors: true });
  infrastructure = composeFixture({
    project,
    file: 'compose.connections.test.yaml',
    root,
  });
  compose(
    'run',
    '--rm',
    '--no-deps',
    '--user',
    '0:0',
    '--entrypoint',
    'chown',
    'server',
    '-R',
    '10001:10001',
    '/var/lib/thelxinoe',
    '/var/cache/thelxinoe',
    '/media',
  );
  compose('up', '-d', '--wait', '--wait-timeout', '180');
  await waitForProxy(context.request, base);
  await api('/setup', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  deployment = (await stack()).deployment_id;
  infrastructure.update({ deployment });
  await install('prowlarr');
  await idle('prowlarr');
  await action('prowlarr', 'stop');
  await install('radarr');
  await verifyNativeProfiles('radarr');
  await waitLink('prowlarr', 'radarr', 'unavailable');
  expect((await link('prowlarr', 'radarr')).enabled).toBe(true);
  const page = await context.newPage();
  await page.goto(`${base}/?section=Settings`);
  await page
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  await page
    .getByRole('navigation', { name: 'Select service' })
    .getByRole('button', { name: 'Prowlarr', exact: true })
    .click();
  const applications = page.getByRole('region', {
    name: 'Applications',
    exact: true,
  });
  await expect(
    applications.getByText('Connection unavailable', { exact: true }),
  ).toBeVisible({ timeout: 15000 });
  await page.screenshot({
    path: `${root}/automatic-connection-outage.png`,
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
    'Automatic connection survives restart; disconnect cleans up with target stopped',
  );

  for (const kind of ['sonarr', 'lidarr']) await install(kind);
  await verifyNativeProfiles('sonarr');
  for (const kind of ['radarr', 'sonarr', 'lidarr']) {
    await idle(kind);
    await action(kind, 'stop');
  }
  await install('nzbget');
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
  for (const kind of ['radarr', 'sonarr', 'lidarr'])
    await action(kind, 'start');
  await install('bazarr');
  compose('restart', 'server');
  await expect
    .poll(
      async () => {
        try {
          return (await links()).length;
        } catch {
          return 0;
        }
      },
      { timeout: 60000 },
    )
    .toBe(8);
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  await waitLink('prowlarr', 'radarr', 'disconnected');
  expect((await links()).filter((l) => l.enabled)).toHaveLength(7);
  for (const item of (await links()).filter((l) => l.enabled))
    await waitLink(item.source_kind, item.target_kind, 'connected');
  expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
  await configure('prowlarr', 'radarr', 'connect');
  for (const item of await links())
    await waitLink(item.source_kind, item.target_kind, 'connected');
  const prefixedApplications = await upstream('prowlarr', 'applications');
  expect(prefixedApplications).toHaveLength(3);
  for (const application of prefixedApplications) {
    expect(
      application.fields
        .find((field) => field.name === 'prowlarrUrl')
        .value.endsWith('/services/prowlarr'),
    ).toBe(true);
    expect(
      application.fields
        .find((field) => field.name === 'baseUrl')
        .value.endsWith(
          `/services/${application.implementation.toLowerCase()}`,
        ),
    ).toBe(true);
  }
  for (const kind of ['radarr', 'sonarr', 'lidarr'])
    expect(await upstream(kind, 'downloadclient')).toHaveLength(1);
  const subtitles = await upstream('bazarr', 'system/settings');
  expect(subtitles.general.base_url.replace(/\/$/, '')).toBe(
    '/services/bazarr',
  );
  expect(subtitles.general.use_radarr).toBe(true);
  expect(subtitles.general.use_sonarr).toBe(true);
  for (const kind of ['radarr', 'sonarr']) {
    expect(subtitles[kind].base_url.replace(/\/$/, '')).toBe(
      `/services/${kind}`,
    );
    const application = (await upstream('prowlarr', 'applications')).find(
      (entry) => entry.implementation.toLowerCase() === kind,
    );
    expect(
      application.fields
        .find((field) => field.name === 'baseUrl')
        .value.endsWith(`/services/${kind}`),
    ).toBe(true);
  }
  const savedCategory = await nzbget('loadconfig');
  expect(savedCategory.find((r) => r.Name === 'Category1.Name').Value).toBe(
    'My custom downloads',
  );
  expect(savedCategory.find((r) => r.Name === 'Category1.DestDir').Value).toBe(
    '/media/downloads/custom',
  );
  console.log(
    'All eight automatic connections verified against real APIs; manual disconnect persists',
  );

  await install('seerr');
  await api('/admin/seerr/sync', 'POST');
  for (const kind of ['radarr', 'sonarr']) {
    const service = (await upstream('seerr', `settings/${kind}`)).find(
      (item) => item.name === `Thelxinoe ${kind}`,
    );
    expect(service.baseUrl).toBe(`/services/${kind}`);
  }
  console.log('Seerr connections use the prefixed Radarr and Sonarr APIs');

  if (!process.argv.includes('--connections-only')) {
    for (const kind of ['lidarr', 'prowlarr', 'bazarr']) {
      const group = kind === 'lidarr' ? 'managers' : 'support';
      const integration = (await api(`/admin/${group}`)).items.find(
        (s) => s.kind === kind,
      );
      const urlBase = `/services/${kind}`;
      if (kind !== 'lidarr') {
        const before = services[kind].container_id;
        await idle(kind);
        await action(kind, 'stop');
        docker('rm', before);
        await action(kind, 'recreate');
        await expect
          .poll(
            async () =>
              (await stack()).provisions.find((p) => p.id === services[kind].id)
                ?.state,
            { timeout: 120000, intervals: [2000] },
          )
          .toBe('complete');
        await refreshService(kind);
        expect(services[kind].container_id).not.toBe(before);
        expect(
          (await api(`/admin/${group}`)).items.find(
            (s) => s.id === integration.id,
          ).url_base,
        ).toBe(urlBase);
      }
      await idle(kind);
      await action(kind, 'stop');
      const update = await api(
        `/admin/service-updates/preflight/${services[kind].id}`,
        'POST',
        {},
      );
      await waitUpdate(update.id, 'ready');
      await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
      await waitUpdate(update.id, 'committed');
      await refreshService(kind);
      expect(services[kind].running).toBe(false);
      expect(
        (await api(`/admin/${group}`)).items.find(
          (s) => s.id === integration.id,
        ).url_base,
      ).toBe(urlBase);
      await action(kind, 'start');
      await expect
        .poll(
          async () => {
            try {
              return (
                await context.request.get(`${base}/services/${kind}`, {
                  maxRedirects: 0,
                })
              ).headers().location;
            } catch {
              return '';
            }
          },
          { timeout: 90000, intervals: [2000] },
        )
        .toBe(`${urlBase}/`);
      for (const item of await links())
        await waitLink(item.source_kind, item.target_kind, 'connected');
      console.log(
        `${kind}: fixed URL Base survives isolated update; all connections recover`,
      );
    }

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
    expect(manager.url_base).toBe('/services/radarr');
    expect((await context.request.get(`${base}/services/radarr`)).url()).toBe(
      `${base}/services/radarr/`,
    );
    await api('/admin/seerr/sync', 'POST');
    expect(
      (await upstream('seerr', 'settings/radarr')).find(
        (item) => item.name === 'Thelxinoe radarr',
      ).baseUrl,
    ).toBe('/services/radarr');
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
      writeFileSync(
        `${root}/interrupted-service.json`,
        JSON.stringify(journal),
      );
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
    await waitUpdate(update.id, 'ready');
    await refreshService('radarr');
    expect(services.radarr.running).toBe(false);
    const preUpdate = services.radarr.container_id;
    await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
    await waitUpdate(update.id, 'committed');
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
    await waitUpdate(update.id, 'ready');
    await refreshService('radarr');
    expect(services.radarr.running).toBe(true);
    await idle('radarr');
    await action('radarr', 'stop');
    await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
    await waitUpdate(update.id, 'committed');
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
      (await api('/admin/managers')).items.find((m) => m.kind === 'radarr')
        .url_base,
    ).toBe('/services/radarr');
    expect(
      (await api('/admin/managers')).items.find((m) => m.kind === 'radarr').id,
    ).toBe(identity);
    expect((await link('prowlarr', 'radarr')).enabled).toBe(false);
    await reconnectSubtitles('radarr');
    // The replacement's own database is fresh; its NZBGet link must still work.
    await configure('radarr', 'nzbget', 'connect');
    await waitLink('radarr', 'nzbget', 'connected');
    const sonarrIdentity = (await api('/admin/managers')).items.find(
      (m) => m.kind === 'sonarr',
    ).id;
    await idle('sonarr');
    await action('sonarr', 'stop');
    docker('rm', services.sonarr.container_id);
    await action('sonarr', 'retire');
    await install('sonarr');
    expect(
      (await api('/admin/managers')).items.find((m) => m.kind === 'sonarr').id,
    ).toBe(sonarrIdentity);
    await reconnectSubtitles('sonarr', true);
    console.log(
      'Recovery after update and retirement preserve appdata; reinstall releases the old reservation',
    );
  }
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify(
      {
        project,
        scope: process.argv.includes('--connections-only')
          ? 'connections'
          : 'full',
        passed: true,
        bootstrap_profiles: bootstrapProfiles,
        retirement_reconnects: retirementReconnects,
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
} catch (error) {
  try {
    writeFileSync(
      `${root}/services.log`,
      compose('logs', '--no-color', 'server', 'controller'),
    );
  } catch (captureError) {
    console.error('Unable to capture service logs:', captureError);
  }
  writeFileSync(
    `${root}/result.json`,
    JSON.stringify(
      {
        project,
        passed: false,
        error: String(error),
        connections: (await links().catch(() => [])).map((l) => ({
          source: l.source_kind,
          target: l.target_kind,
          state: l.state,
          error: l.error,
        })),
      },
      null,
      2,
    ),
  );
  throw error;
} finally {
  try {
    await browser?.close();
  } finally {
    infrastructure?.close();
  }
}
