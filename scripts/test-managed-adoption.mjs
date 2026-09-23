// Isolated real Compose transfer, including copy failure and recovery through the UI.
import { chromium, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { randomBytes, randomUUID, createHash } from 'node:crypto';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
if (
  docker(
    'ps',
    '-aq',
    '--filter',
    'label=com.docker.compose.project=thelxinoe-adoption',
  )
)
  throw Error(
    'An adoption fixture already exists. Inspect it before running another test.',
  );
const run = Date.now();
const project = `thelxinoe-adoption-${run}`;
const root = resolve(`.local/ownership-test-${run}`);
process.env.THELXINOE_ADOPTION_ROOT = root;
for (const folder of [
  'server',
  'cache',
  'radarr',
  'media/movies',
  'media/tv',
  'media/music',
  'media/downloads',
])
  mkdirSync(`${root}/${folder}`, { recursive: true });
const key = randomBytes(16).toString('hex'),
  sentinel = randomUUID();
writeFileSync(
  `${root}/radarr/config.xml`,
  `<Config><BindAddress>*</BindAddress><Port>7878</Port><EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>${key}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><Branch>master</Branch><UpdateAutomatically>False</UpdateAutomatically></Config>`,
);
writeFileSync(`${root}/radarr/ownership-sentinel`, sentinel);
const compose = (...args) =>
  docker('compose', '-p', project, '-f', 'compose.adoption.test.yaml', ...args);
const inspect = (id) => JSON.parse(docker('inspect', id))[0];
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const base = 'https://localhost:25443';
let deployment;
async function response(path, method = 'GET', data) {
  return context.request.fetch(`${base}/api/v1${path}`, {
    method,
    data,
    headers: { 'X-Thelxinoe-Client': '1' },
  });
}
async function api(path, method = 'GET', data) {
  const r = await response(path, method, data);
  if (!r.ok())
    throw Error(`${path}: ${r.status()} ${(await r.json()).error?.message}`);
  return r.json();
}
async function radarr(path, method = 'GET', data) {
  const r = await context.request.fetch(
    `http://localhost:47878/api/v3/${path}`,
    { method, data, headers: { 'X-Api-Key': key } },
  );
  if (!r.ok()) throw Error(`Radarr ${path}: ${r.status()}`);
  return r.json();
}
async function idle() {
  await expect
    .poll(
      async () =>
        (await radarr('command')).filter((c) =>
          ['queued', 'started'].includes(c.status),
        ).length,
      { timeout: 90000 },
    )
    .toBe(0);
}
async function ready() {
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
}
try {
  compose(
    '--profile',
    'external',
    'up',
    '-d',
    '--wait',
    '--wait-timeout',
    '180',
  );
  await api('/setup', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  deployment = (await api('/admin/stack')).deployment_id;
  await ready();
  const original = compose('ps', '-q', 'radarr'),
    old = inspect(original);
  await radarr('rootfolder', 'POST', { path: '/media/movies' });
  const tag = await radarr('tag', 'POST', {
    label: 'keep-my-existing-configuration',
  });
  const integration = await api('/admin/managers', 'POST', {
    name: 'Existing NAS Radarr',
    kind: 'radarr',
    container_id: original,
    port: 7878,
    api_key: key,
  });
  await idle();
  const review = await api('/admin/stack/adopt/preview', 'POST', {
    service_id: integration.id,
  });
  expect(review.compose_project).toBe(project);
  expect(inspect(original).State.Running).toBe(true);
  expect(
    (
      await response('/admin/stack/adopt', 'POST', {
        service_id: integration.id,
        review_id: review.review_id,
      })
    ).status(),
  ).toBe(409);
  docker('update', '--restart', 'on-failure', original);
  expect(
    (
      await response('/admin/stack/adopt', 'POST', {
        service_id: integration.id,
        review_id: review.review_id,
        released_compose: true,
      })
    ).status(),
  ).toBe(409);
  docker('update', '--restart', 'unless-stopped', original);

  // The source Compose service has an opt-in profile, disabled after this startup.
  const page = await context.newPage();
  await page.goto(`${base}/?section=Settings`);
  await page
    .getByRole('button', { name: 'Media services', exact: true })
    .click();
  await page
    .getByLabel('Existing connected service')
    .selectOption(integration.id);
  const previewResponse = page.waitForResponse((r) =>
    r.url().endsWith('/admin/stack/adopt/preview'),
  );
  await page.getByRole('button', { name: 'Review ownership transfer' }).click();
  const failedReview = await (await previewResponse).json();
  await expect(
    page.getByRole('button', { name: 'Stop, copy and take ownership' }),
  ).toBeDisabled();
  await page
    .getByText('I disabled this service in its previous Compose project.', {
      exact: true,
    })
    .click();
  // Force a safe copy failure by occupying the destination in this fixture only.
  const destination = `/var/lib/thelxinoe/deployment/services/${failedReview.review_id}/appdata`;
  compose('exec', '-T', 'controller', 'mkdir', '-p', destination);
  compose('exec', '-T', 'controller', 'touch', `${destination}/occupied`);
  await page
    .getByRole('button', { name: 'Stop, copy and take ownership' })
    .click();
  await expect
    .poll(
      async () =>
        (await api('/admin/stack')).provisions.find(
          (p) => p.id === failedReview.review_id,
        )?.state,
      { timeout: 90000 },
    )
    .toBe('blocked');
  expect(inspect(original).State.Running).toBe(false);
  expect(readFileSync(`${root}/radarr/ownership-sentinel`, 'utf8')).toBe(
    sentinel,
  );
  compose('restart', 'controller');
  await expect
    .poll(
      async () => {
        try {
          return (await api('/admin/stack')).items.some(
            (s) => s.id === failedReview.review_id && s.transfer_pending,
          );
        } catch {
          return false;
        }
      },
      { timeout: 30000 },
    )
    .toBe(true);
  await expect(
    page.getByRole('button', { name: 'Restore original service' }),
  ).toBeVisible({ timeout: 20000 });
  await page.screenshot({
    path: `${root}/failed-copy-recovery.png`,
    fullPage: true,
  });
  await page.getByRole('button', { name: 'Restore original service' }).click();
  await expect
    .poll(async () => (await api('/admin/stack')).provisions.length, {
      timeout: 30000,
    })
    .toBe(0);
  await ready();
  expect(inspect(original).HostConfig.RestartPolicy).toEqual(
    old.HostConfig.RestartPolicy,
  );
  expect(
    (await api('/admin/managers')).items.find((s) => s.id === integration.id)
      .container_id,
  ).toBe(original);
  await idle();

  await page
    .getByLabel('Existing connected service')
    .selectOption(integration.id);
  const successResponse = page.waitForResponse((r) =>
    r.url().endsWith('/admin/stack/adopt/preview'),
  );
  await page.getByRole('button', { name: 'Review ownership transfer' }).click();
  const success = await (await successResponse).json();
  await page
    .getByText('I disabled this service in its previous Compose project.', {
      exact: true,
    })
    .click();
  await page.screenshot({
    path: `${root}/transfer-review.png`,
    fullPage: true,
  });
  await page
    .getByRole('button', { name: 'Stop, copy and take ownership' })
    .click();
  let state;
  await expect
    .poll(
      async () => {
        state = await api('/admin/stack');
        return state.provisions.find((p) => p.id === success.review_id)?.state;
      },
      { timeout: 180000, intervals: [2000] },
    )
    .toBe('complete');
  const managed = state.items.find((s) => s.id === success.review_id),
    current = inspect(managed.container_id);
  expect(current.Image).toBe(old.Image);
  expect(current.Name).toMatch(/^\/thelxinoe-radarr/);
  expect(current.Config.Env).toEqual(old.Config.Env);
  expect(current.HostConfig.PortBindings).toEqual(old.HostConfig.PortBindings);
  expect(
    Object.keys(current.Config.Labels).some((k) =>
      k.startsWith('com.docker.compose.'),
    ),
  ).toBe(false);
  expect(current.Mounts.find((m) => m.Destination === '/config').Source).toBe(
    success.managed_config,
  );
  expect(
    current.Mounts.find((m) => m.Destination === '/config').Source,
  ).not.toBe(old.Mounts.find((m) => m.Destination === '/config').Source);
  expect(current.Mounts.find((m) => m.Destination === '/media').Source).toBe(
    old.Mounts.find((m) => m.Destination === '/media').Source,
  );
  expect(
    docker('exec', managed.container_id, 'cat', '/config/ownership-sentinel'),
  ).toBe(sentinel);
  expect(inspect(original).State.Running).toBe(false);
  expect(inspect(original).HostConfig.RestartPolicy.Name).toBe('no');
  expect(
    (await radarr('tag')).some((t) => t.id === tag.id && t.label === tag.label),
  ).toBe(true);
  expect((await radarr('rootfolder'))[0].path).toBe('/media/movies');
  const connected = (await api('/admin/managers')).items.find(
    (s) => s.id === integration.id,
  );
  expect(connected.name).toBe('Existing NAS Radarr');
  expect(connected.container_id).toBe(managed.container_id);
  const hash = () =>
    createHash('sha256')
      .update(readFileSync(`${root}/radarr/radarr.db`))
      .digest('hex');
  const originalDatabase = hash();
  await radarr('tag', 'POST', { label: 'managed-copy-only' });
  expect(hash()).toBe(originalDatabase);
  await idle();
  await api(`/admin/stack/${managed.id}/action`, 'POST', { action: 'restart' });
  await ready();
  expect(
    (await api('/admin/stack')).items.find((s) => s.id === managed.id).drift,
  ).toBe(false);
  expect(
    (await api('/admin/service-updates')).services.some(
      (s) => s.id === managed.id,
    ),
  ).toBe(true);
  await api(`/admin/service-updates/policy/${managed.id}`, 'POST', {
    policy: 'notify',
    window_start: 0,
    window_end: 0,
  });
  await idle();
  const update = await api(
    `/admin/service-updates/preflight/${managed.id}`,
    'POST',
    {},
  );
  let updateState;
  await expect
    .poll(
      async () => {
        updateState = (await api('/admin/service-updates')).items.find(
          (item) => item.id === update.id,
        );
        return ['ready', 'blocked', 'recovery-required'].includes(
          updateState?.state,
        );
      },
      { timeout: 300000, intervals: [2000] },
    )
    .toBe(true);
  expect({ state: updateState.state, error: updateState.error }).toEqual({
    state: 'ready',
    error: null,
  });
  await ready();
  expect(hash()).toBe(originalDatabase);
  await page.getByRole('button', { name: 'Refresh managed services' }).click();
  await expect(
    page.getByRole('button', { name: 'Refresh managed services' }),
  ).toBeEnabled();
  await page
    .getByRole('article')
    .filter({ hasText: 'Running · active' })
    .scrollIntoViewIfNeeded();
  await page.screenshot({
    path: `${root}/managed-service.png`,
    fullPage: true,
  });
  const result = {
    compose_transfer: true,
    release_confirmation_required: true,
    stale_review_rejected: true,
    copy_failure_recovered: true,
    recovery_survives_controller_restart: true,
    separate_managed_appdata: true,
    original_config_untouched: true,
    original_stopped_restart_disabled: true,
    version_settings_permissions_ports_preserved: true,
    integration_identity_preserved: true,
    managed_restart: true,
    managed_updates_available: true,
    managed_update_preflight: true,
  };
  writeFileSync(`${root}/result.json`, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  console.log(`Evidence: ${root}`);
} catch (error) {
  try {
    writeFileSync(
      `${root}/controller.log`,
      compose('logs', '--no-color', 'controller'),
    );
  } catch {
    /* Startup may have failed before creation. */
  }
  console.error(`Evidence: ${root}`);
  throw error;
} finally {
  await browser.close();
  if (deployment) {
    const ids = docker(
      'ps',
      '-aq',
      '--filter',
      `label=app.thelxinoe.deployment=${deployment}`,
    )
      .split(/\s+/)
      .filter(Boolean);
    if (ids.length) docker('rm', '-f', ...ids);
  }
  compose('--profile', 'external', 'down', '--volumes');
}
