import { spawnSync } from 'node:child_process';
import {
  mkdirSync,
  openSync,
  closeSync,
  readFileSync,
  writeSync,
} from 'node:fs';
import { qualification } from './ci-evidence.mjs';
import {
  stageBuildImages,
  stageServiceImages,
  imageManifestHash,
} from './ci-images.mjs';
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
    group: process.env.THELXINOE_CI_GROUP ?? null,
    log: '.local/ci-command.log',
    started: new Date().toISOString(),
    finished: null,
    passed: null,
  };
  steps.push(step);
  saveSteps();
  const invocation = commandForSpawn(command, args);
  const descriptor = openSync('.local/ci-command.log', 'a');
  let result;
  try {
    writeSync(
      descriptor,
      `\n[${step.started}] ${step.group ?? 'setup'}: ${display}\n`,
    );
    result = spawnSync(invocation.command, invocation.args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: ['inherit', descriptor, descriptor],
    });
  } finally {
    closeSync(descriptor);
  }
  step.finished = new Date().toISOString();
  step.passed = !result.error && result.status === 0;
  saveSteps();
  console.log(
    `${step.passed ? 'PASS' : 'FAIL'} ${display} (${Math.round((Date.parse(step.finished) - Date.parse(step.started)) / 1000)} s)`,
  );
  if (!step.passed)
    console.error(
      readFileSync('.local/ci-command.log', 'utf8')
        .split('\n')
        .slice(-60)
        .join('\n'),
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
  if (
    !process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT &&
    !process.env.CI &&
    !playwrightReady
  ) {
    run(isWindows ? 'pnpm.cmd' : 'pnpm', [
      'exec',
      'playwright',
      'install',
      'chromium',
    ]);
    playwrightReady = true;
  }
}

async function server() {
  if (!process.env.THELXINOE_CI_IN_VERIFY) {
    await evidence.group('image-staging', () =>
      stageBuildImages(['node:24-bookworm-slim', 'rust:1.98-bookworm']),
    );
    await evidence.group(
      'server-verification',
      () => {
        docker(
          'build',
          '-f',
          'scripts/Dockerfile.verify',
          '--target',
          'evidence',
          '--output',
          'type=local,dest=.local/server-verification',
          '--build-arg',
          `REVISION=${evidence.report.revision}`,
          '--build-arg',
          `SOURCE_SHA256=${evidence.report.source_sha256 ?? ''}`,
          '.',
        );
        const result = JSON.parse(
          readFileSync('.local/server-verification/ci-result.json', 'utf8'),
        );
        if (!result.finished || result.passed !== true)
          throw Error(
            'Linux server verification failed; see .local/server-verification/ci-result.json and ci-command.log',
          );
      },
      ['image-staging'],
    );
  } else {
    await evidence.group('rust-format', () =>
      run('cargo', ['fmt', '--all', '--check']),
    );
    await evidence.group('rust-clippy', () =>
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
      ]),
    );
    await evidence.group('rust-product-tests', () =>
      run('cargo', [
        'test',
        '--locked',
        '--workspace',
        '--exclude',
        'thelxinoe-desktop',
      ]),
    );
    await evidence.group('python-product-tests', () =>
      node('scripts/python-tests.mjs'),
    );
  }
}

async function web() {
  ensurePlaywright();
  await evidence.group('web-validation', () => pnpm('run', 'validate:web'));
  await evidence.group(
    'ui',
    () =>
      pnpm('exec', 'playwright', 'test', '--config', 'playwright.ui.config.ts'),
    ['web-validation'],
  );
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
  await evidence.group('image-staging', stageServiceImages);
  await evidence.group(
    'container-build',
    () => {
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
      jsonWrite('.local/ci-images.json', {
        manifest_sha256: imageManifestHash,
        images: identity,
      });
      recordImages({
        imageIds: Object.fromEntries(
          Object.values(identity).map(({ reference, id }) => [reference, id]),
        ),
      });
    },
    ['image-staging'],
  );
  await evidence.group(
    'service-access',
    () => pnpm('run', 'test:service-access', '--built'),
    ['container-build', 'browser-runtime'],
    'test-results/service-access/result.json',
  );
  await evidence.group(
    'service-connections',
    () => node('scripts/test-service-connections.mjs'),
    ['container-build', 'browser-runtime'],
    '.local/connections-result.json',
  );
  await evidence.group(
    'recyclarr',
    () => node('scripts/test-recyclarr.mjs'),
    ['container-build', 'browser-runtime'],
    '.local/recyclarr-result.json',
  );
  await evidence.group(
    'catalog',
    async () => {
      node('scripts/fixtures.mjs');
      docker('compose', 'config', '--quiet');
      process.env.THELXINOE_TEST_HTTP_PORT = String(
        await freePort({ udp: true }),
      );
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
    },
    ['container-build', 'browser-runtime'],
  );
  await evidence.group(
    'playback',
    async () => {
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
    },
    ['container-build', 'browser-runtime'],
  );
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

async function desktop() {
  if (!isWindows) {
    throw new Error('The desktop CI phase requires Windows.');
  }
  await evidence.group('online-storage', () =>
    run('powershell.exe', [
      '-NoProfile',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      'scripts/test-online-storage.ps1',
    ]),
  );
  await evidence.group('desktop-build', () => pnpm('run', 'build:desktop'));
  await evidence.group('desktop-clippy', () =>
    run('cargo', [
      'clippy',
      '--locked',
      '-p',
      'thelxinoe-desktop',
      '--all-targets',
      '--',
      '-D',
      'warnings',
    ]),
  );
  await evidence.group('desktop-product-tests', () =>
    run('cargo', ['test', '--locked', '-p', 'thelxinoe-desktop']),
  );
  await evidence.group('windows-process-isolation', () =>
    run('cargo', [
      'test',
      '--locked',
      '-p',
      'thelxinoe-server',
      'windows_environment_isolated_and_tree_killed',
    ]),
  );
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
async function updatesServer() {
  ensurePlaywright();
  await evidence.group(
    'server-updates',
    () => node('scripts/updates/test.mjs', '--server-only'),
    ['browser-runtime'],
  );
}
async function updatesDesktop() {
  if (!isWindows)
    throw new Error('Desktop update qualification requires Windows.');
  ensurePlaywright();
  await evidence.group('desktop-updates', () =>
    node('scripts/updates/test.mjs', '--desktop-only'),
  );
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
const evidence = qualification(phases);

for (const phase of phases) {
  const execute = phaseFunctions[phase];
  if (!execute) {
    throw new Error(`Unknown CI phase: ${phase}`);
  }
  console.log(`\n=== ${phase} ===`);
  evidence.begin(phase);
  let browser, failure;
  const originalEndpoint = process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT;
  try {
    if (['containers', 'updates-server'].includes(phase)) {
      const resources = process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
      await evidence.group('browser-runtime', async () => {
        if (!originalEndpoint) {
          const { startBrowser } = await import('./ci-browser-runtime.mjs');
          browser = await startBrowser({ resourceDirectory: resources });
        }
        evidence.report.phases.at(-1).browser = {
          image: browser?.image ?? null,
          version: browser?.version ?? evidence.report.environment.playwright,
          transport: 'isolated Docker bridge',
          build_log: '.local/ci-browser-build.log',
          log: '.local/ci-browser.log',
        };
      });
    }
    await execute();
  } catch (error) {
    failure = error;
    console.error(error.message);
  } finally {
    try {
      browser?.close();
    } catch (error) {
      failure ??= error;
    }
    if (originalEndpoint)
      process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT = originalEndpoint;
    else delete process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT;
    const passed = evidence.end(failure);
    results.push({ phase, passed });
    if (!passed) process.exitCode = 1;
  }
}
if (!evidence.finish()) process.exitCode = 1;

console.log('\nCI results:');
for (const { phase, passed } of results)
  console.log(`${passed ? 'PASS' : 'FAIL'} ${phase}`);
