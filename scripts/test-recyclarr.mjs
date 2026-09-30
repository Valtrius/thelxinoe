import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer } from 'node:net';
import { randomUUID } from 'node:crypto';
import { waitForProxy } from './service-access-fixture.mjs';

const project = `thelxinoe-recyclarr-${Date.now()}`;
const adoptionOnly = process.argv.includes('--adoption-only');
const interruptAfterInstall = process.argv.includes(
  '--interrupt-after-install',
);
const root = resolve(`.local/${project}`);
const docker = (...args) => {
  try {
    return execFileSync('docker', args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      maxBuffer: 8 * 1024 * 1024,
      timeout: 360000,
      killSignal: 'SIGKILL',
    }).trim();
  } catch (error) {
    let message = `Docker ${args[0]} failed (${error.code ?? error.status ?? 'unknown'}): ${String(error.stderr ?? '')}`;
    for (const key of keys) message = message.replaceAll(key, '[redacted]');
    // The child-process error includes command arguments containing fixture keys.
    // eslint-disable-next-line preserve-caught-error
    throw Error(message);
  }
};
async function port() {
  const server = createServer();
  await new Promise((done) => server.listen(0, '127.0.0.1', done));
  const value = server.address().port;
  await new Promise((done) => server.close(done));
  return value;
}
process.env.THELXINOE_CONNECTIONS_ROOT = root;
process.env.THELXINOE_CONNECTIONS_PORT = String(await port());
for (const folder of [
  'server',
  'cache',
  'media/movies',
  'media/tv',
  'media/music',
  'media/downloads',
])
  mkdirSync(`${root}/${folder}`, { recursive: true });
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
await context.tracing.start({ screenshots: true, snapshots: true });
const base = `https://localhost:${process.env.THELXINOE_CONNECTIONS_PORT}`;
const services = {};
const keys = [];
const externalContainers = [];
const evidence = {
  project,
  root,
  scope: interruptAfterInstall
    ? 'cleanup'
    : adoptionOnly
      ? 'adoption'
      : 'complete',
  started: new Date().toISOString(),
  finished: null,
  passed: null,
  assertions: [],
  snapshots: {},
};
function saveEvidence() {
  let result = JSON.stringify(evidence, null, 2);
  for (const key of keys) result = result.replaceAll(key, '[redacted]');
  writeFileSync(`${root}/result.json`, result);
  writeFileSync('.local/recyclarr-result.json', result);
}
function record(message) {
  evidence.assertions.push(message);
  saveEvidence();
  console.log(message);
}
saveEvidence();
let deployment;
async function api(path, method = 'GET', data) {
  const response = await context.request.fetch(`${base}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
    timeout: 30000,
  });
  const value = await response
    .json()
    .catch(() => ({ error: { message: 'unavailable' } }));
  if (!response.ok())
    throw Error(`${path}: ${response.status()} ${value.error?.message}`);
  return value;
}
async function install(kind) {
  const host_port = kind === 'recyclarr' ? null : await port();
  const created = await api('/admin/stack/install', 'POST', {
    kind,
    host_port,
  });
  let observed;
  await expect
    .poll(
      async () => {
        observed = await api('/admin/stack');
        const item = observed.provisions.find((p) => p.id === created.id);
        if (item?.state === 'blocked') throw Error(`${kind}: ${item.error}`);
        return item?.state;
      },
      { timeout: 300000, intervals: [1500] },
    )
    .toBe('complete');
  services[kind] = {
    ...observed.items.find((p) => p.id === created.id),
    ...observed.provisions.find((p) => p.id === created.id),
    host_port,
  };
  console.log(`${kind} installed`);
}
async function arr(kind, path, method = 'GET', data) {
  const service = services[kind];
  const config = compose(
    'exec',
    '-T',
    'controller',
    'cat',
    `/var/lib/thelxinoe/deployment/services/${service.id}/appdata/${kind === 'seerr' ? 'settings.json' : 'config.xml'}`,
  );
  const key =
    kind === 'seerr'
      ? JSON.parse(config).main.apiKey
      : config.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  if (!keys.includes(key)) keys.push(key);
  const response = await fetch(
    `http://127.0.0.1:${service.host_port}${kind === 'seerr' ? '' : `/services/${kind}`}/api/v${kind === 'seerr' ? 1 : 3}/${path}`,
    {
      method,
      body: data ? JSON.stringify(data) : undefined,
      headers: { 'X-Api-Key': key, 'Content-Type': 'application/json' },
    },
  );
  if (!response.ok)
    throw Error(`${kind}/${path}: ${response.status} ${await response.text()}`);
  const text = await response.text();
  return text ? JSON.parse(text) : null;
}
function fixtureSQL(sql, values = []) {
  return JSON.parse(
    compose(
      'exec',
      '-T',
      '--user',
      '0',
      'server',
      'python3',
      '-c',
      "import sqlite3,json,sys; c=sqlite3.connect('/var/lib/thelxinoe/thelxinoe.sqlite3'); r=c.execute(sys.argv[1],json.loads(sys.argv[2])).fetchall(); c.commit(); print(json.dumps(r))",
      sql,
      JSON.stringify(values),
    ),
  );
}
async function snapshot(kind) {
  return {
    profiles: await arr(kind, 'qualityprofile'),
    formats: await arr(kind, 'customformat'),
    sizes: await arr(kind, 'qualitydefinition'),
    media: (await arr(kind, kind === 'radarr' ? 'movie' : 'series')).map(
      ({ id, qualityProfileId, path, monitored }) => ({
        id,
        qualityProfileId,
        path,
        monitored,
      }),
    ),
  };
}
const settings = () => api('/admin/recyclarr');
async function waitRun(id, stage = 'complete') {
  let last;
  await expect
    .poll(
      async () => {
        last = (await settings()).runs.find((r) => r.id === id);
        if (['blocked', 'partial'].includes(last?.state))
          throw Error(
            `Run ${last.state}: ${last.error}\n${JSON.stringify(last.evidence)}`,
          );
        return last?.state;
      },
      { timeout: 300000, intervals: [1500] },
    )
    .toBe(stage);
  return last;
}
async function waitUpdate(id, stage) {
  await expect
    .poll(
      async () => {
        const item = (await api('/admin/service-updates')).items.find(
          (u) => u.id === id,
        );
        if (
          [
            'blocked',
            'runtime-failure',
            'recovery-required',
            'rolled-back',
          ].includes(item?.state)
        )
          throw Error(`${item.state}: ${item.error}`);
        return item?.state;
      },
      { timeout: 360000, intervals: [1500] },
    )
    .toBe(stage);
}
try {
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
  deployment = (await api('/admin/stack')).deployment_id;
  evidence.deployment = deployment;
  saveEvidence();
  await install('recyclarr');
  expect(
    JSON.parse(docker('inspect', services.recyclarr.container_id))[0].State
      .Status,
  ).toBe('created');
  if (interruptAfterInstall) {
    await browser.close();
    throw Error('Intentional fixture interruption after installation');
  }
  if (!adoptionOnly) {
    await expect
      .poll(
        async () => {
          try {
            await api(`/admin/stack/${services.recyclarr.id}/action`, 'POST', {
              action: 'start',
            });
            docker('pause', services.recyclarr.container_id);
            return JSON.parse(
              docker('inspect', services.recyclarr.container_id),
            )[0].State.Paused;
          } catch {
            return false;
          }
        },
        { timeout: 45000, intervals: [250] },
      )
      .toBe(true);
    const busyUpdate = await api(
      `/admin/service-updates/preflight/${services.recyclarr.id}`,
      'POST',
      {},
    );
    let busy;
    await expect
      .poll(
        async () => {
          busy = (await api('/admin/service-updates')).items.find(
            (item) => item.id === busyUpdate.id,
          );
          return busy?.state;
        },
        { timeout: 45000 },
      )
      .toBe('blocked');
    expect(busy.error).toMatch(/Stop the Recyclarr job definition/);
    docker('unpause', services.recyclarr.container_id);
    await api(`/admin/stack/${services.recyclarr.id}/action`, 'POST', {
      action: 'stop',
    });
    record(
      'A running job definition blocks preflight with a job-specific idle check; stopping it requires no HTTP integration',
    );
    const emptyUpdate = await api(
      `/admin/service-updates/preflight/${services.recyclarr.id}`,
      'POST',
      {},
    );
    await waitUpdate(emptyUpdate.id, 'ready');
    await api(`/admin/service-updates/${emptyUpdate.id}/activate`, 'POST', {});
    await waitUpdate(emptyUpdate.id, 'committed');
    services.recyclarr.container_id = (await api('/admin/stack')).items.find(
      (p) => p.id === services.recyclarr.id,
    ).container_id;
  }
  expect(services.recyclarr.host_port).toBeNull();
  expect(services.recyclarr.service_id).toBeNull();
  expect(services.recyclarr.status).toBe('ready');
  const raw = JSON.parse(docker('inspect', services.recyclarr.container_id))[0];
  expect(raw.Config.User).toBe('10001:10001');
  expect(Object.keys(raw.HostConfig.PortBindings ?? {})).toHaveLength(0);
  expect(raw.Mounts.map((m) => m.Destination)).toEqual(['/config']);
  expect(raw.Config.Cmd).toEqual(['--version']);
  evidence.image = raw.Config.Image;
  record(
    'Job installs without targets, HTTP integration, port, cron, or media mount; idle is ready',
  );
  await api('/admin/recyclarr/schedule', 'POST', { paused: true, hour: 4 });
  await install('radarr');
  await install('sonarr');
  await expect
    .poll(async () => (await settings()).targets.length, { timeout: 45000 })
    .toBe(2);
  const initialManagers = (await api('/admin/managers')).items;
  for (const kind of ['radarr', 'sonarr']) {
    evidence.snapshots[`${kind}-before`] = await snapshot(kind);
    expect(
      (await settings()).targets.find((t) => t.kind === kind).trash_id,
    ).toBe(
      kind === 'radarr'
        ? 'd1d67249d3890e49bc12e275d989a7e9'
        : '72dae194fc92bf828f32cde7744e51a1',
    );
    const catalog = await api(`/admin/recyclarr/catalog/${kind}`);
    expect(catalog.items.length).toBeGreaterThan(10);
  }
  const preview = await api('/admin/recyclarr/preview', 'POST', {});
  await waitRun(preview.id, 'previewed');
  const previewEvidence = await api(`/admin/recyclarr/runs/${preview.id}`);
  expect(previewEvidence.evidence.before).toEqual(
    previewEvidence.evidence.after,
  );
  expect((await settings()).runs.every((run) => !('evidence' in run))).toBe(
    true,
  );
  for (const kind of ['radarr', 'sonarr'])
    expect(await snapshot(kind)).toEqual(evidence.snapshots[`${kind}-before`]);
  const runs = await Promise.all([
    api('/admin/recyclarr/sync', 'POST', {}),
    api('/admin/recyclarr/sync', 'POST', {}),
  ]);
  expect(runs[0].id).toBe(runs[1].id);
  evidence.run = await waitRun(runs[0].id);
  const current = (await api('/admin/managers')).items;
  for (const kind of ['radarr', 'sonarr']) {
    const target = (await settings()).targets.find((t) => t.kind === kind);
    expect(target.profile_id).toBe(
      current.find((m) => m.kind === kind).defaults.quality_profile,
    );
    expect(target.profile_id).not.toBe(
      initialManagers.find((m) => m.kind === kind).defaults.quality_profile,
    );
    const snapshotValue = await snapshot(kind);
    expect(snapshotValue.formats.length).toBeGreaterThan(8);
    expect(
      snapshotValue.profiles
        .find((p) => p.id === target.profile_id)
        .formatItems.some((f) => f.score > 0),
    ).toBe(true);
    evidence.snapshots[`${kind}-applied`] = snapshotValue;
  }
  await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
  for (const kind of ['radarr', 'sonarr'])
    expect(await snapshot(kind)).toEqual(evidence.snapshots[`${kind}-applied`]);
  record(
    'CLI catalogs contain curated IDs; preview is read-only; sync defaults, CFs and QPs are idempotent; concurrent manual requests deduplicate',
  );
  if (!adoptionOnly) {
    // A real CLI parsing error may exit zero; it must still block before apply.
    const invalidTarget = (await settings()).targets.find(
      (target) => target.kind === 'radarr',
    );
    const invalidManager = (await api('/admin/managers')).items.find(
      (manager) => manager.id === invalidTarget.service_id,
    );
    const nativeMount = JSON.parse(
      docker('inspect', services.radarr.container_id),
    )[0].Mounts.find((mount) => mount.Destination === '/config').Source;
    const invalidCatalog = await api('/admin/recyclarr/catalog/radarr');
    const beforeInvalid = await snapshot('radarr');
    const invalidResult = JSON.parse(
      docker(
        'run',
        '--rm',
        '--network',
        'none',
        '--user',
        '0:0',
        '--entrypoint',
        'python3',
        '--volumes-from',
        `${compose('ps', '-q', 'server')}:ro`,
        '-v',
        `${nativeMount}:/target:ro`,
        'thelxinoe-service-server:local',
        '-c',
        "import json,sys,socket,http.client,xml.etree.ElementTree as E; p=json.loads(sys.argv[2]); p['targets'][0]['secret']=E.parse('/target/config.xml').getroot().findtext('ApiKey'); c=http.client.HTTPConnection('localhost',timeout=300); c.sock=socket.socket(socket.AF_UNIX); c.sock.connect('/run/thelxinoe/controller.sock'); c.request('POST','/stack/'+sys.argv[1]+'/recyclarr/run',json.dumps(p),{'Content-Type':'application/json'}); r=c.getresponse(); assert r.status==200,r.status; print(r.read().decode())",
        services.recyclarr.id,
        JSON.stringify({
          operation_id: randomUUID(),
          image: invalidCatalog.image,
          resources: invalidCatalog.resources,
          preview: false,
          targets: [
            {
              ...invalidTarget,
              container_id: services.radarr.container_id,
              port: invalidManager.port,
              url_base: invalidManager.url_base,
              generation: fixtureSQL(
                'SELECT generation FROM manager_services WHERE id=?',
                [invalidTarget.service_id],
              )[0][0],
              overrides: { min_format_score: 'fixture-invalid' },
              secret: '',
            },
          ],
        }),
      ),
    );
    expect(invalidResult.state).toBe('blocked');
    expect(invalidResult.preview).toContain('[ERR]');
    expect(invalidResult.apply).toBeUndefined();
    expect(await snapshot('radarr')).toEqual(beforeInvalid);
    for (const key of keys)
      expect(JSON.stringify(invalidResult).includes(key)).toBe(false);
    evidence.invalidRun = invalidResult;
    record(
      'CLI errors are detected even with a zero exit status and block application without changing Arr settings',
    );
    const owner = (await settings()).targets.find((t) => t.kind === 'radarr');
    const format = structuredClone((await arr('radarr', 'customformat'))[0]);
    delete format.id;
    format.name = 'Recyclarr fixture unmatched';
    const unmanagedFormat = await arr('radarr', 'customformat', 'POST', format);
    const managedProfile = (await arr('radarr', 'qualityprofile')).find(
      (p) => p.id === owner.profile_id,
    );
    const unmanagedProfile = (await arr('radarr', 'qualityprofile')).find(
      (p) => p.id !== owner.profile_id,
    );
    for (const [profile, score] of [
      [managedProfile, 999],
      [unmanagedProfile, 777],
    ]) {
      const existing = profile.formatItems.find(
        (f) => f.format === unmanagedFormat.id,
      );
      if (existing) existing.score = score;
      else
        profile.formatItems.push({
          format: unmanagedFormat.id,
          name: unmanagedFormat.name,
          score,
        });
      await arr('radarr', `qualityprofile/${profile.id}`, 'PUT', profile);
      expect(
        (await arr('radarr', `qualityprofile/${profile.id}`)).formatItems.find(
          (f) => f.format === unmanagedFormat.id,
        ).score,
      ).toBe(score);
    }
    await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
    const controlled = await arr('radarr', 'qualityprofile');
    expect(
      controlled
        .find((p) => p.id === managedProfile.id)
        .formatItems.find((f) => f.format === unmanagedFormat.id)?.score ?? 0,
    ).toBe(0);
    expect(
      controlled
        .find((p) => p.id === unmanagedProfile.id)
        .formatItems.find((f) => f.format === unmanagedFormat.id).score,
    ).toBe(777);
    expect(
      (await arr('radarr', 'customformat')).some(
        (f) => f.id === unmanagedFormat.id,
      ),
    ).toBe(true);
    for (const kind of ['radarr', 'sonarr'])
      evidence.snapshots[`${kind}-applied`] = await snapshot(kind);
    record(
      'Unmatched scores reset within newly owned profiles; unrelated profiles and custom formats are preserved',
    );
    const collisionGuide = (
      await api('/admin/recyclarr/catalog/radarr')
    ).items.find((g) => g.trash_id !== owner.trash_id);
    const collisionProfile = structuredClone(
      (await arr('radarr', 'qualityprofile')).find(
        (p) => p.id === owner.profile_id,
      ),
    );
    delete collisionProfile.id;
    collisionProfile.name = `Thelxinoe radarr ${collisionGuide.trash_id.slice(0, 8)}`;
    const collision = await arr(
      'radarr',
      'qualityprofile',
      'POST',
      collisionProfile,
    );
    const collisionBefore = await snapshot('radarr');
    const rejectedRun = (
      await api(`/admin/recyclarr/targets/${owner.service_id}`, 'POST', {
        trash_id: collisionGuide.trash_id,
      })
    ).id;
    await expect
      .poll(
        async () =>
          (await settings()).runs.find((r) => r.id === rejectedRun)?.state,
        { timeout: 45000 },
      )
      .toBe('blocked');
    expect(await snapshot('radarr')).toEqual(collisionBefore);
    expect(
      (await api('/admin/managers')).items.find(
        (m) => m.id === owner.service_id,
      ).defaults.quality_profile,
    ).toBe(owner.profile_id);
    await arr('radarr', `qualityprofile/${collision.id}`, 'DELETE');
    await waitRun(
      (
        await api(`/admin/recyclarr/targets/${owner.service_id}`, 'POST', {
          trash_id: owner.trash_id,
        })
      ).id,
    );
    record(
      'An unowned name collision blocks before any Arr writes and retains the previous valid default',
    );
    const page = await context.newPage();
    await page.goto(`${base}/?section=Settings`);
    await page
      .getByRole('button', { name: 'Media services', exact: true })
      .click();
    const navigation = page.getByRole('navigation', { name: 'Select service' });
    for (const kind of ['Radarr', 'Sonarr']) {
      await navigation.getByRole('button', { name: kind, exact: true }).click();
      await expect(
        page.getByLabel('Quality profile', { exact: true }),
      ).toHaveCount(0);
    }
    evidence.ui_catalogs = [];
    evidence.ui_catalog_failures = [];
    let catalogFailuresRemaining = 1;
    let catalogFailureStatus = 0;
    let rejectedCatalogReads = 0;
    page.on('requestfailed', (request) => {
      if (new URL(request.url()).pathname.includes('/recyclarr/catalog/')) {
        evidence.ui_catalog_failures.push({
          path: new URL(request.url()).pathname,
          error: request.failure()?.errorText,
        });
        saveEvidence();
      }
    });
    await page.route(
      '**/api/v1/admin/recyclarr/catalog/radarr',
      async (route) => {
        if (catalogFailureStatus) {
          rejectedCatalogReads++;
          return route.fulfill({
            status: catalogFailureStatus,
            json: {
              error: { code: 'forbidden', message: 'Catalog access denied' },
            },
          });
        }
        if (catalogFailuresRemaining > 0) {
          catalogFailuresRemaining--;
          rejectedCatalogReads++;
          return route.abort('connectionreset');
        }
        return route.continue();
      },
    );
    const guideRegion = page.getByRole('region', {
      name: 'Recyclarr guide configuration',
    });
    const catalogResponses = () =>
      Promise.all(
        ['radarr', 'sonarr'].map((kind) =>
          page
            .waitForResponse(
              (response) =>
                new URL(response.url()).pathname ===
                `/api/v1/admin/recyclarr/catalog/${kind}`,
              { timeout: 180000 },
            )
            .then(async (response) => {
              const catalog = await response.json();
              evidence.ui_catalogs.push({
                path: new URL(response.url()).pathname,
                status: response.status(),
                items: catalog.items?.length,
                error: catalog.error,
              });
              saveEvidence();
              expect(response.ok(), JSON.stringify(catalog.error)).toBe(true);
              expect(catalog.items.length).toBeGreaterThan(10);
            }),
        ),
      );
    await Promise.all([
      catalogResponses(),
      (async () => {
        await navigation
          .getByRole('button', { name: 'Recyclarr', exact: true })
          .click();
        await expect(guideRegion.getByRole('alert')).toContainText(
          'Guide profiles could not be loaded',
        );
      })(),
    ]);
    await expect(guideRegion.getByRole('alert')).toHaveCount(0);
    await expect(
      guideRegion.getByText('TRaSH Guides', { exact: true }),
    ).toBeVisible();
    await expect(
      guideRegion.getByRole('combobox', { name: /^Guide profile for/ }),
    ).toHaveCount(2);
    await expect
      .poll(
        async () =>
          guideRegion
            .getByRole('combobox', { name: /^Guide profile for/ })
            .first()
            .getByRole('option')
            .count(),
        { timeout: 45000 },
      )
      .toBeGreaterThan(10);
    const targetsBeforeCatalogRecovery = (await settings()).targets;
    const waitForStatusPoll = () =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === '/api/v1/admin/recyclarr',
        { timeout: 15000 },
      );
    rejectedCatalogReads = 0;
    catalogFailuresRemaining = Infinity;
    await guideRegion
      .getByRole('button', { name: 'Refresh profiles', exact: true })
      .click();
    await expect.poll(() => rejectedCatalogReads, { timeout: 20000 }).toBe(3);
    for (let poll = 0; poll < 2; poll++) await waitForStatusPoll();
    expect(rejectedCatalogReads).toBe(3);
    await expect(guideRegion.getByRole('alert')).toContainText(
      'Guide profiles could not be loaded',
    );
    await page.screenshot({
      path: `${root}/catalog-retries-exhausted.png`,
      fullPage: true,
    });
    catalogFailuresRemaining = 0;
    await Promise.all([
      catalogResponses(),
      guideRegion
        .getByRole('button', { name: 'Refresh profiles', exact: true })
        .click(),
    ]);
    await expect(guideRegion.getByRole('alert')).toHaveCount(0);
    rejectedCatalogReads = 0;
    catalogFailureStatus = 403;
    await guideRegion
      .getByRole('button', { name: 'Refresh profiles', exact: true })
      .click();
    await expect(guideRegion.getByRole('alert')).toContainText(
      'Catalog access denied',
    );
    for (let poll = 0; poll < 2; poll++) await waitForStatusPoll();
    expect(rejectedCatalogReads).toBe(1);
    await expect(guideRegion.getByRole('alert')).toContainText(
      'Catalog access denied',
    );
    catalogFailureStatus = 0;
    await Promise.all([
      catalogResponses(),
      guideRegion
        .getByRole('button', { name: 'Refresh profiles', exact: true })
        .click(),
    ]);
    await expect(guideRegion.getByRole('alert')).toHaveCount(0);
    expect((await settings()).targets).toEqual(targetsBeforeCatalogRecovery);
    record(
      'Catalog reads recover automatically after a transport failure; exhausted retries and access denials stay visible through status polling and recover on manual refresh without changing guide selections',
    );
    await page.screenshot({
      path: `${root}/recyclarr-profiles.png`,
      fullPage: true,
    });
    await guideRegion.getByText('Sync history', { exact: true }).click();
    const entry = guideRegion.locator('details details').first();
    await entry.locator('summary').click();
    await expect(entry.locator('pre')).toContainText('"config_hash"');
    record(
      'Recyclarr exposes per-instance guide selectors, schedule and history; competing Radarr/Sonarr profile controls are absent',
    );
    await install('seerr');
    await api('/admin/seerr/sync', 'POST', {});
    const assigned = {};
    for (const kind of ['radarr', 'sonarr']) {
      const target = (await settings()).targets.find((t) => t.kind === kind);
      const body =
        kind === 'radarr'
          ? {
              title: 'Recyclarr fixture movie',
              titleSlug: 'recyclarr-fixture',
              tmdbId: 157336,
              year: 2014,
              images: [],
              path: '/media/movies/recyclarr-fixture',
              minimumAvailability: 'released',
              qualityProfileId: target.profile_id,
              monitored: false,
              addOptions: { searchForMovie: false },
            }
          : {
              title: 'Recyclarr fixture series',
              titleSlug: 'recyclarr-fixture',
              tvdbId: 121361,
              year: 2011,
              images: [],
              seasons: [],
              path: '/media/tv/recyclarr-fixture',
              seriesType: 'standard',
              qualityProfileId: target.profile_id,
              monitored: false,
              seasonFolder: true,
              addOptions: { searchForMissingEpisodes: false },
            };
      assigned[kind] = await arr(
        kind,
        kind === 'radarr' ? 'movie' : 'series',
        'POST',
        body,
      );
      const catalog = await api(`/admin/recyclarr/catalog/${kind}`);
      const uhd = catalog.items.find(
        (p) => /UHD|2160p/.test(p.name) && p.trash_id !== target.trash_id,
      );
      expect(uhd).toBeTruthy();
      if (kind === 'sonarr') {
        const saved = page.waitForResponse(
          (r) =>
            r.url().endsWith(`/admin/recyclarr/targets/${target.service_id}`) &&
            r.request().method() === 'POST',
        );
        await guideRegion
          .getByRole('combobox', { name: `Guide profile for ${target.name}` })
          .selectOption(uhd.trash_id);
        const response = await saved;
        expect(response.ok()).toBe(true);
        await waitRun((await response.json()).id);
      } else {
        await waitRun(
          (
            await api(`/admin/recyclarr/targets/${target.service_id}`, 'POST', {
              trash_id: uhd.trash_id,
              quality_sizes: false,
              groups: { add: [], skip: [] },
              overrides: {},
            })
          ).id,
        );
      }
      const changed = (await settings()).targets.find((t) => t.kind === kind);
      expect(changed.profile_id).not.toBe(target.profile_id);
      const existing = await arr(
        kind,
        `${kind === 'radarr' ? 'movie' : 'series'}/${assigned[kind].id}`,
      );
      expect(existing.qualityProfileId).toBe(target.profile_id);
      const defaults = (await api('/admin/managers')).items.find(
        (m) => m.id === target.service_id,
      ).defaults;
      expect(defaults.monitored).toBe(true);
      expect(defaults.quality_profile).toBe(changed.profile_id);
      await expect
        .poll(
          async () =>
            (await arr('seerr', `settings/${kind}`)).find(
              (m) => m.name === `Thelxinoe ${kind}`,
            )?.activeProfileId,
          { timeout: 45000 },
        )
        .toBe(changed.profile_id);
      const configured = (await arr('seerr', `settings/${kind}`)).find(
        (m) => m.name === `Thelxinoe ${kind}`,
      );
      expect(configured.activeProfileName).toBe(changed.profile_name);
      expect(configured.is4k).toBe(true);
      expect(await arr(kind, 'qualitydefinition')).toEqual(
        evidence.snapshots[`${kind}-before`].sizes,
      );
      const deniedGuide = await context.request.post(
        `${base}/api/v1/admin/recyclarr/targets/${target.service_id}`,
        {
          headers: { 'X-Thelxinoe-Client': '1' },
          data: { trash_id: '00000000000000000000000000000000' },
        },
      );
      expect(deniedGuide.status()).toBe(400);
      expect(
        (await settings()).targets.find((t) => t.kind === kind).profile_id,
      ).toBe(changed.profile_id);
      evidence.snapshots[`${kind}-applied`] = await snapshot(kind);
    }
    record(
      'Profile switches affect new defaults only; existing media assignments and global size limits remain; Seerr receives numeric IDs, names and UHD classification; missing selections preserve defaults',
    );
    const optionsTarget = (await settings()).targets.find(
      (t) => t.kind === 'radarr',
    );
    const optionsGuide = (
      await api('/admin/recyclarr/catalog/radarr')
    ).items.find((g) => g.trash_id === optionsTarget.trash_id);
    const extraGroup = optionsGuide.groups.find(
      (g) => g.default === false || g.default === 'false',
    );
    const selectedGroups = {
      add: extraGroup ? [{ trash_id: extraGroup.trash_id }] : [],
      skip: [],
    };
    await waitRun(
      (
        await api(
          `/admin/recyclarr/targets/${optionsTarget.service_id}`,
          'POST',
          {
            trash_id: optionsTarget.trash_id,
            quality_sizes: true,
            groups: selectedGroups,
            overrides: {
              min_format_score: 25,
              upgrade_allowed: false,
              upgrade_until_score: 12345,
            },
          },
        )
      ).id,
    );
    const optionsProfile = (await arr('radarr', 'qualityprofile')).find(
      (p) => p.id === optionsTarget.profile_id,
    );
    expect(optionsProfile.minFormatScore).toBe(25);
    expect(optionsProfile.upgradeAllowed).toBe(false);
    expect(optionsProfile.cutoffFormatScore).toBe(12345);
    await expect
      .poll(() => arr('radarr', 'qualitydefinition'), { timeout: 15000 })
      .not.toEqual(evidence.snapshots['radarr-before'].sizes);
    expect(
      (await settings()).targets.find((t) => t.kind === 'radarr').groups,
    ).toEqual(selectedGroups);
    evidence.snapshots['radarr-applied'] = await snapshot('radarr');
    record(
      'Compatible groups, focused score/upgrade overrides and explicitly enabled global quality-size limits apply',
    );
    // Stop a real target, observe durable bounded retry, and recover without changing defaults.
    docker('stop', services.sonarr.container_id);
    const outage = (await api('/admin/recyclarr/sync', 'POST', {})).id;
    await expect
      .poll(
        async () => (await settings()).runs.find((r) => r.id === outage)?.state,
        { timeout: 45000 },
      )
      .toBe('retrying');
    const duringOutage = (await settings()).targets.find(
      (t) => t.kind === 'sonarr',
    ).profile_id;
    docker('start', services.sonarr.container_id);
    fixtureSQL(
      "UPDATE jobs SET available_at=0 WHERE kind='recyclarr.sync' AND json_extract(payload,'$.id')=?",
      [outage],
    );
    await waitRun(outage);
    expect(
      (await settings()).targets.find((t) => t.kind === 'sonarr').profile_id,
    ).toBe(duringOutage);
    // Advance the persisted schedule clock in the disposable DB to exercise overdue catch-up.
    await api('/admin/recyclarr/schedule', 'POST', { paused: false, hour: 4 });
    const oldRuns = new Set((await settings()).runs.map((r) => r.id));
    fixtureSQL('UPDATE recyclarr_settings SET next_run=0');
    let daily;
    await expect
      .poll(
        async () => {
          daily = (await settings()).runs.find((r) => !oldRuns.has(r.id));
          return !!daily;
        },
        { timeout: 45000 },
      )
      .toBe(true);
    await waitRun(daily.id);
    expect((await settings()).settings.next_run).toBeGreaterThan(
      Date.now() / 1000,
    );
    await api('/admin/recyclarr/schedule', 'POST', { paused: true, hour: 4 });
    record(
      'A real Arr outage retries and recovers with valid defaults; overdue daily schedule catches up once and advances its persisted deadline',
    );
    const priorDefinition = (await api('/admin/stack')).items.find(
      (item) => item.id === services.recyclarr.id,
    ).container_id;
    docker('rm', '-f', priorDefinition);
    await api(`/admin/stack/${services.recyclarr.id}/action`, 'POST', {
      action: 'recreate',
    });
    services.recyclarr.container_id = (await api('/admin/stack')).items.find(
      (item) => item.id === services.recyclarr.id,
    ).container_id;
    expect(services.recyclarr.container_id).not.toBe(priorDefinition);
    expect(
      JSON.parse(docker('inspect', services.recyclarr.container_id))[0].State
        .Status,
    ).toBe('created');
    await expect
      .poll(
        async () =>
          (await api('/admin/stack')).provisions.find(
            (item) => item.id === services.recyclarr.id,
          )?.state,
        { timeout: 45000 },
      )
      .toBe('complete');
    await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'A missing job definition recreates without starting; preserved appdata retains guide mappings and numeric defaults',
    );
    // A real image preflight uses disposable Arr in a loopback-only namespace.
    console.log('Starting Recyclarr image preflight with populated targets');
    const update = await api(
      `/admin/service-updates/preflight/${services.recyclarr.id}`,
      'POST',
      {},
    );
    await waitUpdate(update.id, 'ready');
    console.log('Recyclarr image preflight ready; activating replacement');
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    await api(`/admin/service-updates/${update.id}/activate`, 'POST', {});
    await waitUpdate(update.id, 'committed');
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'Preflight and activation exercise real isolated Arr fixtures without changing production targets',
    );
    compose('restart', 'server', 'controller');
    await waitForProxy(context.request, base);
    await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
    expect(
      (await api('/admin/stack')).items.find((s) => s.kind === 'recyclarr')
        .status,
    ).toBe('ready');
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'Server/controller restart preserves mappings, idle readiness and idempotence',
    );
    for (const kind of ['radarr', 'sonarr'])
      evidence.snapshots[`${kind}-applied`] = await snapshot(kind);
    const interrupted = (await api('/admin/recyclarr/sync', 'POST', {})).id;
    let abandoned;
    await expect
      .poll(
        async () => {
          abandoned = docker(
            'ps',
            '-q',
            '--filter',
            `label=app.thelxinoe.recyclarr-run=${interrupted}`,
            '--filter',
            `ancestor=${evidence.image}`,
          );
          if (!abandoned) return false;
          try {
            docker('pause', abandoned);
            return true;
          } catch (error) {
            if (/is not running|No such container/.test(String(error)))
              return false;
            throw error;
          }
        },
        { timeout: 45000, intervals: [250] },
      )
      .toBe(true);
    compose('restart', 'controller');
    await expect
      .poll(
        async () =>
          (await settings()).runs.find((r) => r.id === interrupted)?.state,
        { timeout: 45000 },
      )
      .toBe('retrying');
    fixtureSQL(
      "UPDATE jobs SET available_at=0 WHERE kind='recyclarr.sync' AND json_extract(payload,'$.id')=?",
      [interrupted],
    );
    await waitRun(interrupted);
    expect(docker('ps', '-aq', '--filter', `id=${abandoned}`)).toBe('');
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'Controller interruption reconciles durable jobs, removes abandoned runtime and key mounts, and preserves profile identities',
    );
    // A persisted administrative actor loses permission before its queued job starts.
    const revokedActor = randomUUID(),
      revokedRun = randomUUID();
    fixtureSQL(
      "INSERT INTO users(id,username,password_hash,role,created_at) SELECT ?, 'revoked-fixture', password_hash, 'admin', unixepoch() FROM users LIMIT 1",
      [revokedActor],
    );
    fixtureSQL(
      "INSERT INTO recyclarr_runs(id,provision_id,actor_id,state,created_at,updated_at) VALUES (?, ?, ?, 'queued',unixepoch(),unixepoch())",
      [revokedRun, services.recyclarr.id, revokedActor],
    );
    fixtureSQL(
      "INSERT INTO jobs(id,kind,payload,dedupe_key,state,available_at,created_at) VALUES (?, 'recyclarr.sync', ?, ?, 'queued', unixepoch()+3,unixepoch())",
      [
        randomUUID(),
        JSON.stringify({ id: revokedRun, preview: false }),
        `recyclarr:${revokedRun}`,
      ],
    );
    fixtureSQL("UPDATE users SET role='user' WHERE id=?", [revokedActor]);
    await expect
      .poll(
        async () =>
          (await settings()).runs.find((r) => r.id === revokedRun)?.state,
        { timeout: 45000 },
      )
      .toBe('blocked');
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'Queued sync rechecks its actor authority and applies nothing after administrator revocation',
    );
    const backupBefore = await settings();
    const backup = await api('/admin/backups', 'POST', {
      passphrase: 'recyclarr fixture backup passphrase',
      confirm: true,
    });
    async function waitBackup(stage) {
      await expect
        .poll(
          async () => {
            try {
              const item = (await api('/admin/backups')).items.find(
                (b) => b.id === backup.id,
              );
              if (item?.error) throw Error(item.error);
              return item?.stage;
            } catch (e) {
              if (/unavailable|ECONN|502|fetch failed/.test(String(e)))
                return 'reconnecting';
              throw e;
            }
          },
          { timeout: 300000, intervals: [2000] },
        )
        .toBe(stage);
    }
    await waitBackup('complete');
    await api('/admin/recyclarr/schedule', 'POST', { paused: true, hour: 17 });
    await api(`/admin/backups/${backup.id}/restore`, 'POST', {
      passphrase: 'recyclarr fixture backup passphrase',
      confirm: true,
    });
    await waitBackup('restored');
    await waitForProxy(context.request, base);
    expect((await settings()).settings.hour).toBe(backupBefore.settings.hour);
    await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
    for (const kind of ['radarr', 'sonarr'])
      expect(await snapshot(kind)).toEqual(
        evidence.snapshots[`${kind}-applied`],
      );
    record(
      'Encrypted deployment backup and restore retain schedule, selections, mapping state, numeric defaults and idempotence',
    );
    const viewer = await browser.newContext({ ignoreHTTPSErrors: true });
    const denied = await viewer.request.post(
      `${base}/api/v1/admin/recyclarr/sync`,
      { headers: { 'X-Thelxinoe-Client': '1' }, data: {} },
    );
    expect(denied.status()).toBe(401);
    await viewer.close();
    const payloads = compose(
      'exec',
      '-T',
      '--user',
      '0',
      'server',
      'python3',
      '-c',
      "import sqlite3,glob; c=sqlite3.connect('/var/lib/thelxinoe/thelxinoe.sqlite3'); print(c.execute(\"select payload from jobs where kind='recyclarr.sync'\").fetchall())",
    );
    for (const key of keys) expect(payloads.includes(key)).toBe(false);
    const serialized = JSON.stringify(await settings());
    for (const key of keys) expect(serialized.includes(key)).toBe(false);
    record(
      'Unauthorized sync is rejected; queued payloads and public evidence contain no API keys',
    );
  }
  // Create an independently owned copy with a real cron process for reviewed transfer.
  const ownedRaw = JSON.parse(
    docker(
      'inspect',
      (await api('/admin/stack')).items.find((s) => s.kind === 'recyclarr')
        .container_id,
    ),
  )[0];
  const volume = `${root}/external-recyclarr`;
  mkdirSync(volume, { recursive: true });
  docker(
    'run',
    '--rm',
    '--network',
    'none',
    '--user',
    '0:0',
    '--entrypoint',
    '/usr/local/bin/thelxinoe-docker-controller',
    '-v',
    `${ownedRaw.Mounts.find((m) => m.Destination === '/config').Source}:/source:ro`,
    '-v',
    `${volume}:/destination`,
    'thelxinoe-service-controller:local',
    'snapshot-copy',
  );
  const privateYaml = compose(
    'exec',
    '-T',
    'controller',
    'cat',
    `/var/lib/thelxinoe/deployment/services/${services.recyclarr.id}/appdata/recyclarr.yml`,
  );
  let originalYaml = privateYaml.replaceAll('enabled: true', 'enabled: false');
  for (const kind of ['radarr', 'sonarr']) {
    const manager = (await api('/admin/managers')).items.find(
      (m) => m.kind === kind,
    );
    const xml = compose(
      'exec',
      '-T',
      'controller',
      'cat',
      `/var/lib/thelxinoe/deployment/services/${services[kind].id}/appdata/config.xml`,
    );
    originalYaml = originalYaml.replace(
      `!file /runtime/${manager.id}`,
      JSON.stringify(xml.match(/<ApiKey>(.*?)<\/ApiKey>/)[1]),
    );
  }
  function externalConfig(yaml, settingsText = '{}') {
    docker(
      'run',
      '--rm',
      '--network',
      'none',
      '--user',
      '0:0',
      '--entrypoint',
      'python3',
      '-v',
      `${volume}:/config`,
      'thelxinoe-service-server:local',
      '-c',
      "import sys,os; open('/config/recyclarr.yml','w').write(sys.argv[1]); open('/config/settings.yml','w').write(sys.argv[2]); os.chown('/config/recyclarr.yml',10001,10001); os.chown('/config/settings.yml',10001,10001)",
      yaml,
      settingsText,
    );
  }
  externalConfig(originalYaml);
  const external = docker(
    'run',
    '-d',
    '--name',
    `${project}-external`,
    '--network',
    `${project}_test`,
    '--user',
    '10001:10001',
    '-e',
    'CRON_SCHEDULE=0 0 1 1 *',
    '-v',
    `${volume}:/config`,
    evidence.image,
  );
  externalContainers.push(external);
  await api(`/admin/stack/${services.recyclarr.id}/action`, 'POST', {
    action: 'remove',
  });
  expect(
    (await api('/admin/stack')).items.some((s) => s.kind === 'recyclarr'),
  ).toBe(false);
  record('Removal cleans up the owned job definition and state');
  externalConfig(
    originalYaml,
    'resource_providers: [{name: custom, type: trash-guides, path: /custom}]',
  );
  const unsupported = await context.request.post(
    `${base}/api/v1/admin/recyclarr/adopt/preview`,
    {
      headers: { 'X-Thelxinoe-Client': '1' },
      data: { container_id: external },
    },
  );
  expect(unsupported.status()).toBe(409);
  expect(JSON.parse(docker('inspect', external))[0].State.Running).toBe(true);
  externalConfig(originalYaml);
  const stale = await api('/admin/recyclarr/adopt/preview', 'POST', {
    container_id: external,
  });
  externalConfig(`${originalYaml}\n# changed after review\n`);
  const rejected = await context.request.post(
    `${base}/api/v1/admin/recyclarr/adopt`,
    {
      headers: { 'X-Thelxinoe-Client': '1' },
      data: {
        review_id: stale.review_id,
        container_id: external,
        released_compose: true,
      },
    },
  );
  expect(rejected.status()).toBe(409);
  expect(JSON.parse(docker('inspect', external))[0].State.Running).toBe(true);
  externalConfig(originalYaml);
  const review = await api('/admin/recyclarr/adopt/preview', 'POST', {
    container_id: external,
  });
  for (const key of keys)
    expect(JSON.stringify(review).includes(key)).toBe(false);
  const adopted = await api('/admin/recyclarr/adopt', 'POST', {
    review_id: review.review_id,
    container_id: external,
    released_compose: true,
  });
  let adoptedStack;
  await expect
    .poll(
      async () => {
        adoptedStack = await api('/admin/stack');
        const provision = adoptedStack.provisions.find(
          (p) => p.id === adopted.id,
        );
        if (provision?.state === 'blocked') throw Error(provision.error);
        return provision?.state;
      },
      { timeout: 180000 },
    )
    .toBe('complete');
  services.recyclarr = {
    ...adoptedStack.items.find((p) => p.id === adopted.id),
    ...adoptedStack.provisions.find((p) => p.id === adopted.id),
  };
  expect((await settings()).settings.paused).toBe(true);
  expect(JSON.parse(docker('inspect', external))[0].State.Running).toBe(false);
  expect(
    JSON.parse(docker('inspect', external))[0].HostConfig.RestartPolicy.Name,
  ).toBe('no');
  const managedYaml = compose(
    'exec',
    '-T',
    'controller',
    'cat',
    `/var/lib/thelxinoe/deployment/services/${adopted.id}/appdata/recyclarr.yml`,
  );
  for (const key of keys) expect(managedYaml.includes(key)).toBe(false);
  await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
  for (const kind of ['radarr', 'sonarr'])
    expect(await snapshot(kind)).toEqual(evidence.snapshots[`${kind}-applied`]);
  record(
    'Unsupported or stale adoption preserves the original; reviewed transfer stops cron, copies and scrubs appdata, starts paused and retains mappings',
  );
  await api(`/admin/stack/${services.recyclarr.id}/action`, 'POST', {
    action: 'remove',
  });

  evidence.passed = true;
  console.log(`Recyclarr E2E passed: ${root}`);
} catch (error) {
  evidence.passed = false;
  evidence.error = String(error);
  throw error;
} finally {
  evidence.cleanup_errors = [];
  saveEvidence();
  async function cleanup(name, action) {
    let timer;
    try {
      return await Promise.race([
        Promise.resolve().then(action),
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(Error(`${name} timed out`)), 30000);
        }),
      ]);
    } catch (error) {
      evidence.cleanup_errors.push(`${name}: ${String(error)}`);
      console.error(`Cleanup failed: ${name}`);
    } finally {
      clearTimeout(timer);
    }
  }
  await cleanup('service logs', () => {
    let logs = compose('logs', '--no-color');
    for (const key of keys) logs = logs.replaceAll(key, '[redacted]');
    writeFileSync(`${root}/services.log`, logs);
  });
  if (!evidence.passed)
    await cleanup('failure screenshot', () =>
      context
        .pages()
        .at(-1)
        ?.screenshot({
          path: `${root}/failure.png`,
          fullPage: true,
          timeout: 10000,
        }),
    );
  await cleanup('browser trace', () =>
    context.tracing.stop({ path: `${root}/trace.zip` }),
  );
  await cleanup('browser close', () => browser.close());
  const containers = deployment
    ? await cleanup('owned container inventory', () =>
        docker(
          'ps',
          '-aq',
          '--filter',
          `label=app.thelxinoe.deployment=${deployment}`,
        ),
      )
    : '';
  for (const container of (containers ?? '').split(/\s+/).filter(Boolean))
    await cleanup(`owned container ${container}`, () =>
      docker('rm', '-f', container),
    );
  for (const container of externalContainers)
    await cleanup(`external fixture ${container}`, () =>
      docker('rm', '-f', container),
    );
  await cleanup('compose deployment', () =>
    compose('down', '--volumes', '--remove-orphans'),
  );
  evidence.finished = new Date().toISOString();
  if (evidence.cleanup_errors.length) {
    evidence.passed = false;
    process.exitCode = 1;
  }
  saveEvidence();
}
