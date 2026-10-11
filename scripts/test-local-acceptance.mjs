// Real Compose: accepting rebuilt images and changed configuration after the first start.
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  composeFixture,
  fixtureId,
  freePort,
  resourceRecord,
} from './ci-resources.mjs';

const serverImage = process.env.THELXINOE_SERVER_IMAGE;
const controllerImage = process.env.THELXINOE_CONTROLLER_IMAGE;
if (!serverImage || !controllerImage)
  throw Error(
    'Set THELXINOE_SERVER_IMAGE and THELXINOE_CONTROLLER_IMAGE to built images',
  );
const project = fixtureId('acceptance');
const root = resolve(`.local/${project}`);
const output = '.local/local-acceptance-result.json';
const result = { passed: false, checks: [] };
const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 360000,
  }).trim();
const ensure = (condition, message) => {
  if (!condition) throw Error(message);
};
const check = (name) => {
  result.checks.push(name);
  console.log(`PASS ${name}`);
};
for (const directory of ['server', 'cache', 'media', 'backups'])
  mkdirSync(`${root}/${directory}`, { recursive: true });
// Linux bind mounts need the runner's own identity; Docker Desktop accepts any.
const uid = process.getuid?.() || 10001;
const gid = process.getgid?.() || 10001;
const fixture = composeFixture({
  project,
  root,
  file: 'compose.yaml',
  env: {
    ...process.env,
    THELXINOE_UID: String(uid),
    THELXINOE_GID: String(gid),
    THELXINOE_STATE_ROOT: root,
    THELXINOE_MEDIA_ROOT: `${root}/media`,
    THELXINOE_BACKUP_ROOT: `${root}/backups`,
    THELXINOE_LISTEN: '127.0.0.1',
    THELXINOE_SERVER_IMAGE: serverImage,
    THELXINOE_CONTROLLER_IMAGE: controllerImage,
    THELXINOE_PUBLIC_URL: '',
    THELXINOE_TRUSTED_PROXIES: '',
    THELXINOE_DISCOVERY: 'false',
    THELXINOE_DISCOVERY_URL: '',
    THELXINOE_CORS_ORIGINS: '',
  },
});
const config = fixture.config;
const write = () =>
  writeFileSync(`${root}/compose.json`, JSON.stringify(config, null, 2));
const port = await freePort();
for (const [name, service] of Object.entries(config.services)) {
  delete service.build;
  // The production names are fixed; each run keeps its own.
  service.container_name = `${project}-${name}`;
}
// The deployment record needs a Linux filesystem with symlinks on every host.
config.services.controller.volumes = config.services.controller.volumes.map(
  (mount) =>
    mount.target === '/var/lib/thelxinoe/deployment'
      ? { type: 'volume', source: 'deployment', target: mount.target }
      : mount,
);
config.volumes.deployment = {
  labels: { 'io.thelxinoe.ci-run': process.env.THELXINOE_CI_RUN_ID },
};
config.services.server.ports = [
  {
    target: 8484,
    published: String(port),
    host_ip: '127.0.0.1',
    protocol: 'tcp',
  },
];
write();
const attempt = (...command) =>
  spawnSync('docker', [...fixture.args, ...command], {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 360000,
  });
const accept = () =>
  attempt('run', '--rm', '-T', 'controller', 'accept-local', '--yes');
const said = (outcome) => `${outcome.stdout ?? ''}${outcome.stderr ?? ''}`;
const deployment = () =>
  JSON.parse(
    docker(
      'exec',
      `${project}-controller`,
      'curl',
      '-fsS',
      '--unix-socket',
      '/run/thelxinoe/controller.sock',
      'http://localhost/stack',
    ),
  ).deployment_id;
const start = () =>
  fixture.compose('up', '-d', '--wait', '--wait-timeout', '180');
let images, cleanup;
try {
  start();
  const id = deployment();
  ensure(id, 'The deployment was not initialized');
  let outcome = accept();
  ensure(
    outcome.status === 0 && said(outcome).includes('Nothing to accept'),
    `Unchanged containers needed acceptance: ${said(outcome)}`,
  );
  check('accepted containers need no acceptance');

  // A rebuild gives both components new image IDs.
  images = resourceRecord({ images: [], closed: false });
  const rebuilt = {};
  for (const [name, image] of [
    ['server', serverImage],
    ['controller', controllerImage],
  ]) {
    rebuilt[name] = `${project}-${name}:${process.env.THELXINOE_CI_RUN_ID}`;
    execFileSync('docker', ['build', '-t', rebuilt[name], '-'], {
      input: `FROM ${image}\nLABEL io.thelxinoe.ci-run=${process.env.THELXINOE_CI_RUN_ID}\n`,
      windowsHide: true,
      timeout: 300000,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
  }
  images({ images: Object.values(rebuilt) });
  config.services.server.image = rebuilt.server;
  config.services.controller.image = rebuilt.controller;
  write();
  const waiting = attempt('up', '-d', '--wait', '--wait-timeout', '60');
  ensure(waiting.status !== 0, 'Rebuilt containers started without acceptance');
  const logs = spawnSync('docker', ['logs', `${project}-controller`], {
    encoding: 'utf8',
    windowsHide: true,
  });
  ensure(
    said(logs).includes('controller accept-local'),
    `The waiting controller did not explain acceptance: ${said(logs)}`,
  );
  outcome = accept();
  for (const expected of [
    'Accepted as generation',
    'server Image changed',
    'controller Image changed',
  ])
    ensure(
      outcome.status === 0 && said(outcome).includes(expected),
      `Rebuild acceptance missing "${expected}": ${said(outcome)}`,
    );
  console.log(said(outcome).trim());
  start();
  ensure(deployment() === id, 'A rebuild started a different deployment');
  check('a rebuild is accepted into the same deployment');

  // Only the server changes, so the accepted controller keeps running.
  config.services.server.environment.THELXINOE_DISCOVERY_URL =
    'http://192.0.2.10:8484';
  write();
  start();
  const began = Date.now();
  outcome = accept();
  ensure(
    outcome.status !== 0 &&
      said(outcome).includes('docker compose stop controller') &&
      Date.now() - began < 30000,
    `A running controller's deployment was not refused promptly: ${said(outcome)}`,
  );
  fixture.compose('stop', 'controller');
  outcome = accept();
  ensure(
    outcome.status === 0 &&
      said(outcome).includes(
        'server Config.Env.THELXINOE_DISCOVERY_URL changed',
      ),
    `Server change acceptance failed: ${said(outcome)}`,
  );
  console.log(said(outcome).trim());
  start();
  ensure(deployment() === id, 'A server change started a different deployment');
  check('a server-only change is accepted after stopping the controller');
  result.passed = true;
} finally {
  writeFileSync(output, JSON.stringify(result, null, 2));
  for (const record of [fixture, images])
    try {
      record?.close();
    } catch (error) {
      cleanup ??= error;
    }
}
// A failed scenario is reported first; cleanup failures fail an otherwise passing run.
if (cleanup) throw cleanup;
