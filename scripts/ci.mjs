import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { requestedPhases } from './ci-phases.mjs';

const isWindows = process.platform === 'win32';
let playwrightReady = false;
const steps = [];
function saveSteps() {
  mkdirSync('.local', { recursive: true });
  writeFileSync('.local/ci-steps.json', JSON.stringify(steps, null, 2) + '\n');
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

function cleanup(command, args, options = {}) {
  try {
    run(command, args, options);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
  }
}

function npm(...args) {
  run(isWindows ? 'npm.cmd' : 'npm', args);
}

function node(script, ...args) {
  run(process.execPath, [script, ...args]);
}

function docker(...args) {
  run('docker', args);
}

function ensurePlaywright() {
  if (!process.env.CI && !playwrightReady) {
    run(isWindows ? 'npx.cmd' : 'npx', ['playwright', 'install', 'chromium']);
    playwrightReady = true;
  }
}

function server() {
  if (isWindows) {
    docker(
      'build',
      '-f',
      'scripts/Dockerfile.verify',
      '-t',
      'thelxinoe-verified:local',
      '.',
    );
  } else {
    npm('run', 'check:rust');
    npm('run', 'test:rust');
  }
  if (!isWindows) npm('run', 'test:python');
}

function web() {
  ensurePlaywright();
  npm('run', 'validate:web');
  npm('run', 'test:ui:layout');
  npm('run', 'test:ui:player');
}

function containers() {
  const playbackProject = process.env.COMPOSE_PROJECT_NAME
    ? `${process.env.COMPOSE_PROJECT_NAME}-playback`
    : 'thelxinoe-playback';
  ensurePlaywright();
  const version = JSON.parse(readFileSync('package.json', 'utf8')).version;
  run(isWindows ? 'npm.cmd' : 'npm', ['run', 'build:containers'], {
    env: {
      ...process.env,
      THELXINOE_SERVER_IMAGE: `thelxinoe-server:${version}`,
      THELXINOE_CONTROLLER_IMAGE: `thelxinoe-controller:${version}`,
    },
  });
  for (const component of ['server', 'controller'])
    docker(
      'tag',
      `thelxinoe-${component}:${version}`,
      `thelxinoe-service-${component}:local`,
    );
  npm('run', 'test:service-access', '--', '--built');
  node('scripts/test-service-connections.mjs');
  cleanup('docker', [
    'compose',
    '-f',
    'compose.test.yaml',
    'down',
    '--volumes',
    '--remove-orphans',
  ]);
  cleanup('docker', [
    'compose',
    '-p',
    playbackProject,
    '-f',
    'compose.test.yaml',
    'down',
    '--volumes',
    '--remove-orphans',
  ]);
  node('scripts/fixtures.mjs');
  docker('compose', 'config', '--quiet');

  try {
    docker('compose', '-f', 'compose.test.yaml', 'up', '-d', '--wait');
    populateMedia(['compose', '-f', 'compose.test.yaml']);
    run(isWindows ? 'npm.cmd' : 'npm', ['run', 'test:e2e'], {
      env: {
        ...process.env,
        THELXINOE_PROXY_TEST: '1',
        THELXINOE_TEST_URL: `https://localhost:${process.env.THELXINOE_TEST_HTTPS_PORT ?? '9443'}`,
      },
    });
  } finally {
    cleanup('docker', [
      'compose',
      '-f',
      'compose.test.yaml',
      'down',
      '--volumes',
      '--remove-orphans',
    ]);
  }

  node('scripts/playback-fixtures.mjs');
  const playbackEnvironment = {
    ...process.env,
    THELXINOE_TEST_HTTP_PORT:
      process.env.THELXINOE_PLAYBACK_HTTP_PORT ?? '18686',
    THELXINOE_TEST_HTTPS_PORT:
      process.env.THELXINOE_PLAYBACK_HTTPS_PORT ?? '20443',
    THELXINOE_PLAYBACK_URL: `https://localhost:${process.env.THELXINOE_PLAYBACK_HTTPS_PORT ?? '20443'}`,
    THELXINOE_TEST_SUBNET: '172.31.252.0/24',
  };
  try {
    run(
      'docker',
      [
        'compose',
        '-p',
        playbackProject,
        '-f',
        'compose.test.yaml',
        'up',
        '-d',
        '--wait',
      ],
      { env: playbackEnvironment },
    );
    populateMedia([
      'compose',
      '-p',
      playbackProject,
      '-f',
      'compose.test.yaml',
    ]);
    for (const script of [
      'test-playback.mjs',
      'test-playback-tracks.mjs',
      'test-user-media.mjs',
    ])
      run(process.execPath, ['scripts/' + script], {
        env: playbackEnvironment,
      });
  } finally {
    cleanup('docker', [
      'compose',
      '-p',
      playbackProject,
      '-f',
      'compose.test.yaml',
      'down',
      '--volumes',
      '--remove-orphans',
    ]);
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
  npm('run', 'build:desktop');
  run('cargo', [
    'clippy',
    '--workspace',
    '--all-targets',
    '--',
    '-D',
    'warnings',
  ]);
  run('cargo', ['test', '--locked', '-p', 'thelxinoe-desktop']);
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
    execute();
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
