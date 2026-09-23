import { spawnSync } from 'node:child_process';

const isWindows = process.platform === 'win32';
let playwrightReady = false;

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
  const invocation = commandForSpawn(command, args);
  const result = spawnSync(invocation.command, invocation.args, {
    cwd: options.cwd,
    env: options.env ?? process.env,
    stdio: 'inherit',
  });
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
  npm('run', 'test:python');
}

function web() {
  ensurePlaywright();
  npm('run', 'validate:web');
  npm('run', 'test:ui:layout');
  npm('run', 'test:ui:player');
}

function containers() {
  ensurePlaywright();
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
    'thelxinoe-playback',
    '-f',
    'compose.test.yaml',
    'down',
    '--volumes',
    '--remove-orphans',
  ]);
  node('scripts/fixtures.mjs');
  npm('run', 'build:containers');
  docker('compose', 'config', '--quiet');

  try {
    docker('compose', '-f', 'compose.test.yaml', 'up', '-d', '--wait');
    run(isWindows ? 'npm.cmd' : 'npm', ['run', 'test:e2e'], {
      env: {
        ...process.env,
        THELXINOE_PROXY_TEST: '1',
        THELXINOE_TEST_URL: 'https://localhost:9443',
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
    THELXINOE_TEST_HTTP_PORT: '18686',
    THELXINOE_TEST_HTTPS_PORT: '20443',
    THELXINOE_TEST_SUBNET: '172.31.252.0/24',
  };
  try {
    run(
      'docker',
      [
        'compose',
        '-p',
        'thelxinoe-playback',
        '-f',
        'compose.test.yaml',
        'up',
        '-d',
        '--wait',
      ],
      { env: playbackEnvironment },
    );
    node('scripts/test-playback.mjs');
    node('scripts/test-playback-tracks.mjs');
    node('scripts/test-user-media.mjs');
  } finally {
    cleanup('docker', [
      'compose',
      '-p',
      'thelxinoe-playback',
      '-f',
      'compose.test.yaml',
      'down',
      '--volumes',
      '--remove-orphans',
    ]);
  }
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
}

const requested = process.argv.slice(2);
const phases = requested.length
  ? requested
  : isWindows
    ? ['server', 'web', 'containers', 'desktop']
    : ['server', 'web', 'containers'];
const phaseFunctions = { server, web, containers, desktop };

for (const phase of phases) {
  const execute = phaseFunctions[phase];
  if (!execute) {
    throw new Error(`Unknown CI phase: ${phase}`);
  }
  console.log(`\n=== ${phase} ===`);
  execute();
}
