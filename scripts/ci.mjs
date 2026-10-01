import { spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { requestedPhases } from './ci-phases.mjs';
import { atomicWrite, jsonWrite } from './ci-state.mjs';
import {
  composeFixture,
  fixtureId,
  fixtureImages,
  freePort,
  resourceRecord,
} from './ci-resources.mjs';

const isWindows = process.platform === 'win32';
let playwrightReady = false;
const steps = [];
function saveSteps() {
  mkdirSync('.local', { recursive: true });
  atomicWrite('.local/ci-steps.json', JSON.stringify(steps, null, 2) + '\n');
}

function commandForSpawn(command, args) {
  if (isWindows && command.toLowerCase().endsWith('.cmd')) {
    return {
      command: process.env.ComSpec ?? 'cmd.exe',
      args: ['/d', '/s', '/c', [command, ...args].join(' ')],
    };
  }
  return { command, args };
}

function run(command, args, options = {}) {
  const display = [command, ...args].join(' ');
  console.log(`\n> ${display}`);
  const step = {
    command: display,
    started: new Date().toISOString(),
    finished: null,
    passed: null,
  };
  steps.push(step);
  saveSteps();
  const invocation = commandForSpawn(command, args);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: options.cwd,
    env: options.env ?? process.env,
    stdio: 'inherit',
  });
  step.finished = new Date().toISOString();
  step.passed = !result.error && result.status === 0;
  saveSteps();
  console.log(
    `${step.passed ? 'PASS' : 'FAIL'} ${display} (${Math.round((Date.parse(step.finished) - Date.parse(step.started)) / 1000)} s)`,
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${display} exited with code ${result.status ?? 1}`);
  }
}

function pnpm(...args) {
  run(isWindows ? 'pnpm.cmd' : 'pnpm', args);
}

function node(script, ...args) {
  run(process.execPath, [script, ...args]);
}

function docker(...args) {
  run('docker', args);
}

function ensurePlaywright() {
  if (!process.env.CI && !playwrightReady) {
    run(isWindows ? 'pnpm.cmd' : 'pnpm', [
      'exec',
      'playwright',
      'install',
      'chromium',
    ]);
    playwrightReady = true;
  }
}

function server() {
  if (isWindows) {
    docker('build', '-f', 'scripts/Dockerfile.verify', '.');
  } else {
    run('cargo', ['fmt', '--all', '--check']);
    run('cargo', [
      'clippy',
      '--locked',
      '--workspace',
      '--exclude',
      'thelxinoe-desktop',
      '--all-targets',
      '--',
      '-D',
      'warnings',
    ]);
    run('cargo', [
      'test',
      '--locked',
      '--workspace',
      '--exclude',
      'thelxinoe-desktop',
    ]);
  }
  if (!isWindows) node('scripts/python-tests.mjs');
}

function web() {
  ensurePlaywright();
  pnpm('run', 'validate:web');
  pnpm('exec', 'playwright', 'test', '--config', 'playwright.ui.config.ts');
}

async function containers() {
  const project = fixtureId('catalog');
  const playbackProject = fixtureId('playback');
  process.env.COMPOSE_PROJECT_NAME = project;
  ensurePlaywright();
  const images = fixtureImages();
  process.env.THELXINOE_SERVER_IMAGE = images.server;
  process.env.THELXINOE_CONTROLLER_IMAGE = images.controller;
  const recordImages = resourceRecord({
    images: Object.values(images),
    closed: false,
  });
  pnpm('run', 'build:containers');
  const identity = {};
  for (const [component, reference] of Object.entries(images)) {
    const result = spawnSync(
      'docker',
      ['image', 'inspect', '--format', '{{.Id}}', reference],
      { encoding: 'utf8', windowsHide: true },
    );
    if (result.status !== 0) throw Error(`Could not inspect ${reference}`);
    identity[component] = { reference, id: result.stdout.trim() };
  }
  jsonWrite('.local/ci-images.json', identity);
  recordImages({
    imageIds: Object.fromEntries(
      Object.values(identity).map(({ reference, id }) => [reference, id]),
    ),
  });
  pnpm('run', 'test:service-access', '--built');
  node('scripts/test-service-connections.mjs');
  node('scripts/test-recyclarr.mjs');
  node('scripts/fixtures.mjs');
  docker('compose', 'config', '--quiet');
  process.env.THELXINOE_TEST_HTTP_PORT = String(await freePort({ udp: true }));
  process.env.THELXINOE_TEST_HTTPS_PORT = String(await freePort());
  const catalog = composeFixture({
    project,
    file: 'compose.test.yaml',
    root: `.local/${project}`,
  });
  try {
    docker(...catalog.args, 'up', '-d', '--wait');
    populateMedia(catalog.args);
    run(isWindows ? 'pnpm.cmd' : 'pnpm', ['run', 'test:e2e'], {
      env: {
        ...process.env,
        THELXINOE_PROXY_TEST: '1',
        THELXINOE_TEST_URL: `https://localhost:${process.env.THELXINOE_TEST_HTTPS_PORT ?? '9443'}`,
      },
    });
  } finally {
    catalog.close();
  }

  node('scripts/playback-fixtures.mjs');
  const playbackEnvironment = {
    ...process.env,
    THELXINOE_TEST_HTTP_PORT: String(await freePort({ udp: true })),
    THELXINOE_TEST_HTTPS_PORT: String(await freePort()),
    COMPOSE_PROJECT_NAME: playbackProject,
  };
  playbackEnvironment.THELXINOE_PLAYBACK_URL = `https://localhost:${playbackEnvironment.THELXINOE_TEST_HTTPS_PORT}`;
  const playback = composeFixture({
    project: playbackProject,
    file: 'compose.test.yaml',
    root: `.local/${playbackProject}`,
    env: playbackEnvironment,
  });
  try {
    run('docker', [...playback.args, 'up', '-d', '--wait'], {
      env: playbackEnvironment,
    });
    populateMedia(playback.args);
    for (const script of [
      'test-playback.mjs',
      'test-playback-tracks.mjs',
      'test-user-media.mjs',
    ])
      run(process.execPath, ['scripts/' + script], {
        env: playbackEnvironment,
      });
  } finally {
    playback.close();
  }
}

function populateMedia(compose) {
  docker(...compose, 'cp', '.local/fixtures/.', 'server:/media');
  docker(
    ...compose,
    'exec',
    '-T',
    '--user',
    '0',
    'server',
    'chown',
    '-R',
    '10001:10001',
    '/media',
  );
}

function desktop() {
  if (!isWindows) {
    throw new Error('The desktop CI phase requires Windows.');
  }
  run('powershell.exe', [
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    'scripts/test-online-storage.ps1',
  ]);
  pnpm('run', 'build:desktop');
  run('cargo', [
    'clippy',
    '--locked',
    '-p',
    'thelxinoe-desktop',
    '--all-targets',
    '--',
    '-D',
    'warnings',
  ]);
  run('cargo', ['test', '--locked', '-p', 'thelxinoe-desktop']);
  run('cargo', [
    'test',
    '--locked',
    '-p',
    'thelxinoe-server',
    'windows_environment_isolated_and_tree_killed',
  ]);
}

const requested = process.argv.slice(2);
if (!process.env.CI && requested.length === 0) {
  if (isWindows)
    run('powershell.exe', [
      '-NoProfile',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      'scripts/ci-local.ps1',
    ]);
  else node('scripts/ci-local.mjs');
  process.exit(0);
}
function updatesServer() {
  ensurePlaywright();
  node('scripts/updates/test.mjs', '--server-only');
}
function updatesDesktop() {
  if (!isWindows)
    throw new Error('Desktop update qualification requires Windows.');
  ensurePlaywright();
  node('scripts/updates/test.mjs', '--desktop-only');
}
const phases = requestedPhases(requested);
const phaseFunctions = {
  server,
  web,
  containers,
  desktop,
  'updates-server': updatesServer,
  'updates-desktop': updatesDesktop,
};
const results = [];

for (const phase of phases) {
  const execute = phaseFunctions[phase];
  if (!execute) {
    throw new Error(`Unknown CI phase: ${phase}`);
  }
  console.log(`\n=== ${phase} ===`);
  try {
    await execute();
    results.push({ phase, passed: true });
  } catch (error) {
    console.error(error);
    results.push({ phase, passed: false });
    process.exitCode = 1;
  }
}
console.log('\nCI results:');
for (const { phase, passed } of results)
  console.log(`${passed ? 'PASS' : 'FAIL'} ${phase}`);
