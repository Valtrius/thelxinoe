import { expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { composeFixture, fixtureId, freePort } from './ci-resources.mjs';
import { randomUUID } from 'node:crypto';
import { waitForProxy } from './service-access-fixture.mjs';
import {
  requestBudget,
  waitForProvision,
  waitForJob,
} from './ci-readiness.mjs';
import { launchBrowser } from './ci-browser.mjs';

const project = fixtureId('recyclarr');
const adoptionOnly = process.argv.includes('--adoption-only');
const profilesOnly = process.argv.includes('--profiles-only');
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
process.env.THELXINOE_CONNECTIONS_ROOT = root;
process.env.THELXINOE_CONNECTIONS_PORT = String(await freePort());
for (const folder of [
  'server',
  'cache',
  'media/movies',
  'media/tv',
  'media/music',
  'media/downloads',
])
  mkdirSync(`${root}/${folder}`, { recursive: true });
let infrastructure;
const compose = (...args) => infrastructure.compose(...args);
let browser, context;
const base = `https://localhost:${process.env.THELXINOE_CONNECTIONS_PORT}`;
const services = {};
const keys = [];
const externalContainers = [];
const evidence = {
  project,
  root,
  browser_network: process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT
    ? 'isolated Docker bridge'
    : 'host',
  scope: profilesOnly ? 'profiles' : adoptionOnly ? 'adoption' : 'complete',
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
async function request(
  path,
  method = 'GET',
  data,
  timeout = requestBudget(path),
) {
  // Restore verifies a full archive, and both adoption endpoints copy appdata
  // before responding (even when rejecting a stale or unsupported review).
  const copiesState = requestBudget(path) > requestBudget('/setup');
  const started = Date.now();
  let status = null;
  try {
    const response = await context.request.fetch(`${base}/api/v1${path}`, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
      timeout,
    });
    status = response.status();
    return response;
  } finally {
    if (copiesState) {
      (evidence.state_requests ??= []).push({
        path,
        method,
        status,
        elapsed_ms: Date.now() - started,
      });
      saveEvidence();
    }
  }
}
async function api(path, method = 'GET', data, timeout) {
  const response = await request(path, method, data, timeout);
  const value = await response
    .json()
    .catch(() => ({ error: { message: 'unavailable' } }));
  if (!response.ok())
    throw Error(`${path}: ${response.status()} ${value.error?.message}`);
  return value;
}
async function install(kind) {
  const host_port = kind === 'recyclarr' ? null : await freePort();
  const created = await api('/admin/stack/install', 'POST', {
    kind,
    host_port,
  });
  let observed;
  observed = (
    await waitForProvision(
      (timeout) => api('/admin/stack', 'GET', undefined, timeout),
      created.id,
      kind,
    )
  ).stack;
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
async function defaultTarget(kind) {
  const [state, managers] = await Promise.all([
    settings(),
    api('/admin/managers'),
  ]);
  const manager = managers.items.find((m) => m.kind === kind);
  return (
    state.targets.find(
      (t) =>
        t.service_id === manager.id &&
        t.profile_id === manager.defaults.quality_profile,
    ) ?? state.targets.find((t) => t.service_id === manager.id)
  );
}
async function waitRun(id, stage = 'complete') {
  return waitForJob(
    async (timeout) =>
      (await api('/admin/recyclarr', 'GET', undefined, timeout)).runs.find(
        (r) => r.id === id,
      ),
    stage,
    `Recyclarr run ${id}`,
  );
}
async function waitUpdate(id, stage) {
  return waitForJob(
    async (timeout) =>
      (
        await api('/admin/service-updates', 'GET', undefined, timeout)
      ).items.find((u) => u.id === id),
    stage,
    `Service update ${id}`,
    {
      timeout: 360000,
      failures: [
        'blocked',
        'runtime-failure',
        'recovery-required',
        'rolled-back',
      ],
    },
  );
}
scenario: try {
  browser = await launchBrowser();
  context = await browser.newContext({
    ignoreHTTPSErrors: true,
    viewport: { width: 1440, height: 1000 },
  });
  await context.tracing.start({ screenshots: true, snapshots: true });
  infrastructure = composeFixture({
    project,
    file: 'compose.connections.test.yaml',
    root,
  });
  compose(
    'run',
    '--label',
    `com.docker.compose.project=${project}`,
    '--label',
    `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
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
  infrastructure.update({ deployment });
  saveEvidence();
  await install('recyclarr');
  expect(
    JSON.parse(docker('inspect', services.recyclarr.container_id))[0].State
      .Status,
  ).toBe('created');
  if (!adoptionOnly && !profilesOnly) {
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
    expect((await defaultTarget(kind)).trash_id).toBe(
      kind === 'radarr'
        ? '05fbf054ac8ad0303335026cc2632f1a'
        : 'c4cadd6b35b95f62c3d47a408e53e2f7',
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
    const catalog = await api(`/admin/recyclarr/catalog/${kind}`);
    const targets = (await settings()).targets.filter((t) => t.kind === kind);
    expect(targets.map((t) => t.trash_id).sort()).toEqual(
      catalog.items.map((p) => p.trash_id).sort(),
    );
    const manager = current.find((m) => m.kind === kind);
    const previous = initialManagers.find((m) => m.kind === kind).defaults
      .quality_profile;
    if (previous) expect(manager.defaults.quality_profile).toBe(previous);
    const snapshotValue = await snapshot(kind);
    expect(snapshotValue.formats.length).toBeGreaterThan(8);
    for (const target of targets) {
      expect(target.profile_id).toBeGreaterThan(0);
      expect(
        snapshotValue.profiles.some((p) => p.id === target.profile_id),
      ).toBe(true);
    }
    const combined = targets.find(
      (t) =>
        t.trash_id ===
        (kind === 'radarr'
          ? '05fbf054ac8ad0303335026cc2632f1a'
          : 'c4cadd6b35b95f62c3d47a408e53e2f7'),
    );
    expect(
      snapshotValue.profiles
        .find((p) => p.id === combined.profile_id)
        .formatItems.some((f) => f.score > 0),
    ).toBe(true);
    await api(`/admin/managers/${manager.id}/defaults`, 'PUT', {
      ...manager.defaults,
      quality_profile: combined.profile_id,
    });
    evidence.snapshots[`${kind}-applied`] = snapshotValue;
  }
  await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
  for (const kind of ['radarr', 'sonarr'])
    expect(await snapshot(kind)).toEqual(evidence.snapshots[`${kind}-applied`]);
  record(
    'The full official catalog and profile scores sync idempotently while preserving defaults; concurrent manual requests deduplicate; internal preview is read-only',
  );
  if (!adoptionOnly) {
    // A real CLI parsing error may exit zero; it must still block before apply.
    const invalidTarget = await defaultTarget('radarr');
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
        '--label',
        `com.docker.compose.project=${project}`,
        '--label',
        `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
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
        process.env.THELXINOE_SERVER_IMAGE ?? 'thelxinoe-service-server:local',
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
    const owner = await defaultTarget('radarr');
    const format = structuredClone((await arr('radarr', 'customformat'))[0]);
    delete format.id;
    format.name = 'Recyclarr fixture unmatched';
    const unmanagedFormat = await arr('radarr', 'customformat', 'POST', format);
    const managedProfile = (await arr('radarr', 'qualityprofile')).find(
      (p) => p.id === owner.profile_id,
    );
    const managedIds = new Set(
      (await settings()).targets
        .filter((t) => t.kind === 'radarr')
        .map((t) => t.profile_id),
    );
    const unmanagedProfile = (await arr('radarr', 'qualityprofile')).find(
      (p) => !managedIds.has(p.id),
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
    const collisionTarget = (await settings()).targets.find(
      (t) => t.kind === 'radarr' && t.trash_id === collisionGuide.trash_id,
    );
    await arr(
      'radarr',
      `qualityprofile/${collisionTarget.profile_id}`,
      'DELETE',
    );
    const collisionProfile = structuredClone(
      (await arr('radarr', 'qualityprofile')).find(
        (p) => p.id === owner.profile_id,
      ),
    );
    delete collisionProfile.id;
    collisionProfile.name = collisionGuide.name;
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
      const profiles = page.getByRole('combobox', {
        name: 'Default request profile',
        exact: true,
      });
      const mapped = (await settings()).targets.filter(
        (t) => t.kind === kind.toLowerCase(),
      );
      await expect(profiles).toBeVisible();
      await expect
        .poll(() => profiles.getByRole('option').count())
        .toBeGreaterThan(mapped.length);
      await expect(
        page.getByRole('link', { name: 'View TRaSH profile', exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole('link', {
          name: `Manage profiles in ${kind}`,
          exact: true,
        }),
      ).toBeVisible();
      await page.screenshot({
        path: `${root}/${kind.toLowerCase()}-profiles.png`,
        fullPage: true,
      });
      await page.setViewportSize({ width: 390, height: 844 });
      await profiles.scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${root}/${kind.toLowerCase()}-profiles-mobile.png`,
        fullPage: true,
      });
      await page.setViewportSize({ width: 1440, height: 1000 });
    }
    const guideRegion = page.getByRole('region', {
      name: 'Recyclarr guide configuration',
    });
    await navigation
      .getByRole('button', { name: 'Recyclarr', exact: true })
      .click();
    await expect(
      guideRegion.getByRole('button', { name: 'Sync now', exact: true }),
    ).toBeVisible();
    await expect(
      guideRegion.getByRole('switch', {
        name: 'Pause automatic sync',
        exact: true,
      }),
    ).toBeVisible();
    await expect(
      guideRegion.getByRole('combobox', { name: /^Daily sync hour/ }),
    ).toBeVisible();
    await expect(guideRegion.getByRole('combobox')).toHaveCount(1);
    await expect(guideRegion.getByRole('link')).toHaveCount(0);
    await expect(
      guideRegion.getByText(
        /Preview|Sync history|Last sync|Next sync|Synced profiles/,
      ),
    ).toHaveCount(0);
    await page.screenshot({
      path: `${root}/recyclarr-settings.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await guideRegion.getByRole('combobox').scrollIntoViewIfNeeded();
    await page.screenshot({
      path: `${root}/recyclarr-settings-mobile.png`,
      fullPage: true,
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    const runCount = (await settings()).runs.length;
    await guideRegion
      .getByRole('button', { name: 'Sync now', exact: true })
      .click();
    await expect
      .poll(
        async () => {
          const runs = (await settings()).runs;
          return runs.length > runCount && runs[0].state === 'complete';
        },
        { timeout: 300000, intervals: [1500] },
      )
      .toBe(true);
    await expect(
      guideRegion.getByRole('button', { name: 'Sync now', exact: true }),
    ).toBeEnabled();
    await expect(guideRegion.getByRole('status')).toHaveCount(0);
    expect((await settings()).runs.length).toBeGreaterThan(runCount);
    expect(
      fixtureSQL(
        "SELECT count(*) FROM audit WHERE action='recyclarr.complete'",
      )[0][0],
    ).toBeGreaterThan(0);
    record(
      'Radarr and Sonarr expose their full profile lists and native management links; Recyclarr contains only manual sync and scheduling, with results in global history',
    );
    await install('seerr');
    await api('/admin/seerr/sync', 'POST', {});
    const assigned = {};
    for (const kind of ['radarr', 'sonarr']) {
      const target = await defaultTarget(kind);
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
      const selected = (await settings()).targets.find(
        (t) =>
          t.service_id === target.service_id && t.trash_id === uhd.trash_id,
      );
      const manager = (await api('/admin/managers')).items.find(
        (m) => m.id === target.service_id,
      );
      if (kind === 'sonarr') {
        await navigation
          .getByRole('button', { name: 'Sonarr', exact: true })
          .click();
        const saved = page.waitForResponse(
          (r) =>
            r.url().endsWith(`/admin/managers/${target.service_id}/defaults`) &&
            r.request().method() === 'PUT',
        );
        await page
          .getByRole('combobox', {
            name: 'Default request profile',
            exact: true,
          })
          .selectOption(String(selected.profile_id));
        expect((await saved).ok()).toBe(true);
      } else {
        await api(`/admin/managers/${target.service_id}/defaults`, 'PUT', {
          ...manager.defaults,
          quality_profile: selected.profile_id,
        });
      }
      await waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id);
      const changed = await defaultTarget(kind);
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
      expect((await defaultTarget(kind)).profile_id).toBe(changed.profile_id);
      evidence.snapshots[`${kind}-applied`] = await snapshot(kind);
    }
    record(
      'Profile switches affect new defaults only; existing media assignments and global size limits remain; Seerr receives numeric IDs, names and UHD classification; missing selections preserve defaults',
    );
    const requestDefault = await defaultTarget('radarr');
    const sonarrDefault = await defaultTarget('sonarr');
    const choices = await api('/seerr/profiles/movie?media_id=11');
    expect(choices.items[0]).toMatchObject({
      default: true,
      profile: {
        source: 'guide',
        service_id: requestDefault.service_id,
        trash_id: requestDefault.trash_id,
      },
    });
    expect(
      choices.items.filter((p) => p.profile.source === 'guide'),
    ).toHaveLength((await api('/admin/recyclarr/catalog/radarr')).items.length);
    const seriesChoices = await api('/seerr/profiles/tv?media_id=1399');
    expect(seriesChoices.locked).toBe(true);
    for (const [profile, status] of [
      [
        {
          source: 'guide',
          service_id: sonarrDefault.service_id,
          trash_id: sonarrDefault.trash_id,
        },
        400,
      ],
      [
        {
          source: 'guide',
          service_id: requestDefault.service_id,
          trash_id: '00000000000000000000000000000000',
        },
        409,
      ],
      [
        {
          source: 'custom',
          service_id: requestDefault.service_id,
          profile_id: requestDefault.profile_id,
        },
        409,
      ],
      [
        {
          source: 'custom',
          service_id: requestDefault.service_id,
          profile_id: 999999,
        },
        409,
      ],
    ]) {
      const denied = await context.request.post(
        `${base}/api/v1/seerr/requests`,
        {
          headers: { 'X-Thelxinoe-Client': '1' },
          data: { media_type: 'movie', media_id: 11, seasons: [], profile },
        },
      );
      expect(denied.status()).toBe(status);
    }
    const changedSeries = await context.request.post(
      `${base}/api/v1/seerr/requests`,
      {
        headers: { 'X-Thelxinoe-Client': '1' },
        data: {
          media_type: 'tv',
          media_id: 1399,
          seasons: [1],
          profile: {
            source: 'guide',
            service_id: sonarrDefault.service_id,
            trash_id: sonarrDefault.trash_id,
          },
        },
      },
    );
    expect(changedSeries.status()).toBe(409);
    const defaultRequest = await api('/seerr/requests', 'POST', {
      media_type: 'movie',
      media_id: 11,
      seasons: [],
    });
    expect(defaultRequest).toMatchObject({
      profileId: requestDefault.profile_id,
      is4k: true,
    });
    expect((await api('/seerr/movie/11')).mediaInfo.status).toBeGreaterThan(1);
    const hd = (await settings()).targets.find(
      (t) =>
        t.kind === 'radarr' &&
        t.trash_id === 'd1d67249d3890e49bc12e275d989a7e9',
    );
    const renamed = await arr('radarr', `qualityprofile/${hd.profile_id}`);
    renamed.name = 'Renamed fixture guide';
    await arr('radarr', `qualityprofile/${hd.profile_id}`, 'PUT', renamed);
    const guideRequest = await api('/seerr/requests', 'POST', {
      media_type: 'movie',
      media_id: 238,
      seasons: [],
      profile: {
        source: 'guide',
        service_id: hd.service_id,
        trash_id: hd.trash_id,
      },
    });
    expect(guideRequest).toMatchObject({
      profileId: hd.profile_id,
      is4k: false,
    });
    const custom = structuredClone(renamed);
    delete custom.id;
    custom.name = 'Native custom fixture';
    const nativeCustom = await arr('radarr', 'qualityprofile', 'POST', custom);
    const customRequest = await api('/seerr/requests', 'POST', {
      media_type: 'movie',
      media_id: 550,
      seasons: [],
      profile: {
        source: 'custom',
        service_id: hd.service_id,
        profile_id: nativeCustom.id,
      },
    });
    expect(customRequest).toMatchObject({
      profileId: nativeCustom.id,
      is4k: false,
    });
    expect((await defaultTarget('radarr')).profile_id).toBe(
      requestDefault.profile_id,
    );
    const managerDefaults = (await api('/admin/managers')).items.find(
      (m) => m.id === hd.service_id,
    ).defaults;
    let reconciliation;
    docker('pause', services.seerr.container_id);
    try {
      const started = Date.now();
      await api(
        `/admin/managers/${hd.service_id}/defaults`,
        'PUT',
        {
          ...managerDefaults,
          quality_profile: nativeCustom.id,
        },
        5000,
      );
      evidence.defaults_while_seerr_unresponsive = {
        elapsed_ms: Date.now() - started,
        quality_profile: (await api('/admin/managers')).items.find(
          (m) => m.id === hd.service_id,
        ).defaults.quality_profile,
      };
      expect(evidence.defaults_while_seerr_unresponsive.quality_profile).toBe(
        nativeCustom.id,
      );
      reconciliation = api('/admin/managers/reconcile', 'POST', {});
    } finally {
      docker('unpause', services.seerr.container_id);
    }
    record('Saving defaults remains responsive while Seerr is unresponsive');
    await Promise.all([
      reconciliation,
      waitRun((await api('/admin/recyclarr/sync', 'POST', {})).id),
    ]);
    expect(
      (await api('/admin/managers')).items.find((m) => m.id === hd.service_id)
        .defaults.quality_profile,
    ).toBe(nativeCustom.id);
    expect(
      (await api('/seerr/profiles/movie?media_id=680')).items[0],
    ).toMatchObject({
      default: true,
      profile: {
        source: 'custom',
        service_id: hd.service_id,
        profile_id: nativeCustom.id,
      },
    });
    await api(
      `/admin/managers/${hd.service_id}/defaults`,
      'PUT',
      managerDefaults,
    );
    for (const kind of ['radarr', 'sonarr'])
      evidence.snapshots[`${kind}-applied`] = await snapshot(kind);
    record(
      'Seerr receives default, guide-ID and native-ID profile choices per request; invalid and cross-manager profiles are rejected; existing series are locked; guide renames and custom defaults survive sync',
    );
    if (profilesOnly) {
      evidence.passed = true;
      console.log(`Profile E2E passed: ${root}`);
      break scenario;
    }
    const optionsTarget = await defaultTarget('radarr');
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
    expect((await defaultTarget('radarr')).groups).toEqual(selectedGroups);
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
    const duringOutage = (await defaultTarget('sonarr')).profile_id;
    docker('start', services.sonarr.container_id);
    fixtureSQL(
      "UPDATE jobs SET available_at=0 WHERE kind='recyclarr.sync' AND json_extract(payload,'$.id')=?",
      [outage],
    );
    await waitRun(outage);
    expect((await defaultTarget('sonarr')).profile_id).toBe(duringOutage);
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
    evidence.backup_id = backup.id;
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
    const restoreStarted = Date.now();
    // The controller decrypts and verifies the entire archive before accepting
    // restore. Managed appdata on a bind mount can exceed the normal API budget.
    try {
      await api(`/admin/backups/${backup.id}/restore`, 'POST', {
        passphrase: 'recyclarr fixture backup passphrase',
        confirm: true,
      });
    } finally {
      evidence.restore_request_ms = Date.now() - restoreStarted;
      saveEvidence();
    }
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
    '--label',
    `com.docker.compose.project=${project}`,
    '--label',
    `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
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
    process.env.THELXINOE_CONTROLLER_IMAGE ??
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
      '--label',
      `com.docker.compose.project=${project}`,
      '--label',
      `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}`,
      '--rm',
      '--network',
      'none',
      '--user',
      '0:0',
      '--entrypoint',
      'python3',
      '-v',
      `${volume}:/config`,
      process.env.THELXINOE_SERVER_IMAGE ?? 'thelxinoe-service-server:local',
      '-c',
      "import sys,os; open('/config/recyclarr.yml','w').write(sys.argv[1]); open('/config/settings.yml','w').write(sys.argv[2]); os.chown('/config/recyclarr.yml',10001,10001); os.chown('/config/settings.yml',10001,10001)",
      yaml,
      settingsText,
    );
  }
  externalConfig(originalYaml);
  externalContainers.push(`${project}-external`);
  infrastructure.update({ containers: externalContainers });
  const external = docker(
    'run',
    '-d',
    '--label',
    `io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID ?? project}`,
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
  const unsupported = await request('/admin/recyclarr/adopt/preview', 'POST', {
    container_id: external,
  });
  expect(unsupported.status()).toBe(409);
  expect(JSON.parse(docker('inspect', external))[0].State.Running).toBe(true);
  externalConfig(originalYaml);
  const stale = await api('/admin/recyclarr/adopt/preview', 'POST', {
    container_id: external,
  });
  externalConfig(`${originalYaml}\n# changed after review\n`);
  const rejected = await request('/admin/recyclarr/adopt', 'POST', {
    review_id: stale.review_id,
    container_id: external,
    released_compose: true,
  });
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
  const adoptedStack = (
    await waitForProvision(
      (timeout) => api('/admin/stack', 'GET', undefined, timeout),
      adopted.id,
      'Recyclarr adoption',
    )
  ).stack;
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
  evidence.product_passed = evidence.passed;
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
  if (evidence.backup_id)
    await cleanup('backup state', () => {
      const state = JSON.parse(
        compose(
          'exec',
          '-T',
          'controller',
          'curl',
          '-fsS',
          '--max-time',
          '10',
          '--unix-socket',
          '/run/thelxinoe/controller.sock',
          'http://localhost/stack/backups',
        ),
      );
      evidence.backup = state.items.find(
        (item) => item.id === evidence.backup_id,
      );
    });
  if (!evidence.passed)
    await cleanup('failure screenshot', () =>
      context
        ?.pages()
        .at(-1)
        ?.screenshot({
          path: `${root}/failure.png`,
          fullPage: true,
          timeout: 10000,
        }),
    );
  await cleanup('browser trace', () =>
    context?.tracing.stop({ path: `${root}/trace.zip` }),
  );
  await cleanup('browser close', () => browser?.close());
  await cleanup('compose deployment', () => infrastructure?.close());
  evidence.finished = new Date().toISOString();
  if (evidence.cleanup_errors.length) {
    evidence.passed = false;
    process.exitCode = 1;
  }
  saveEvidence();
}
