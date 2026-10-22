import { qualifyHostRecovery } from './auth-host-recovery.mjs';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { delimiter, dirname, join, resolve } from 'node:path';
import { stageBuildImages, stageServiceImages } from './ci-images.mjs';
import {
  cleanupResources,
  composeFixture,
  fixtureId,
  fixtureImages,
  freePort,
  resourceRecord,
  resourceScope,
} from './ci-resources.mjs';

const isWindows = process.platform === 'win32';
const githubActions = !!process.env.GITHUB_ACTIONS;
const pnpmCommand = isWindows ? 'pnpm.cmd' : 'pnpm';

function run(command, args, options = {}) {
  const display = [command, ...args].join(' ');
  console.log(`\n> ${display}`);
  const started = Date.now();
  const invocation =
    isWindows && command.endsWith('.cmd')
      ? {
          command: process.env.ComSpec ?? 'cmd.exe',
          args: ['/d', '/s', '/c', [command, ...args].join(' ')],
        }
      : { command, args };
  const result = spawnSync(invocation.command, invocation.args, {
    env: options.env ?? process.env,
    stdio: 'inherit',
  });
  const passed = !result.error && result.status === 0;
  console.log(
    `${passed ? 'PASS' : 'FAIL'} ${display} (${Math.round((Date.now() - started) / 1000)} s)`,
  );
  if (result.error) throw result.error;
  if (!passed)
    throw new Error(`${display} exited with code ${result.status ?? 1}`);
}

const pnpm = (...args) => run(pnpmCommand, args);
const node = (script, ...args) => run(process.execPath, [script, ...args]);
const docker = (...args) => run('docker', args);

const failures = [];
/**
 * Run one check and remove the fixture resources it leaves behind. A failure
 * is reported at the end so later, independent checks still run.
 */
async function group(name, action) {
  const parent = process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
  const resources = join(parent, name);
  process.env.THELXINOE_CI_RESOURCE_DIRECTORY = resources;
  if (githubActions) console.log(`::group::${name}`);
  const started = Date.now();
  try {
    await action();
  } catch (error) {
    failures.push(name);
    console.error(`${name}: ${error.message}`);
  } finally {
    try {
      browserRuntime?.close();
    } catch (error) {
      failures.push(`browser cleanup: ${error.message}`);
    }
    const errors = cleanupResources(resources, process.env.THELXINOE_CI_RUN_ID);
    if (errors.length) {
      failures.push(`${name} cleanup`);
      console.error(errors.join('\n'));
    }
    process.env.THELXINOE_CI_RESOURCE_DIRECTORY = parent;
    if (githubActions) console.log('::endgroup::');
    console.log(
      `${failures.includes(name) ? 'FAIL' : 'PASS'} ${name} (${Math.round((Date.now() - started) / 1000)} s)`,
    );
  }
}

function ensurePlaywright() {
  if (!process.env.CI && !process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT)
    pnpm('exec', 'playwright', 'install', 'chromium');
}

/** Start the pinned browser on an isolated Docker bridge for container fixtures. */
let browserRuntime;
async function browser() {
  if (process.env.THELXINOE_CI_BROWSER_WS_ENDPOINT) return;
  const { startBrowser } = await import('./ci-browser-runtime.mjs');
  browserRuntime = await startBrowser();
}

const imageExists = (reference) =>
  spawnSync('docker', ['image', 'inspect', reference], { stdio: 'ignore' })
    .status === 0;

/**
 * This revision's server and controller images. CI builds them once and loads
 * them into each container job; a local run builds them when they are missing.
 */
async function images() {
  const references = {
    server: process.env.THELXINOE_SERVER_IMAGE,
    controller: process.env.THELXINOE_CONTROLLER_IMAGE,
  };
  const named = !references.server || !references.controller;
  if (named) Object.assign(references, fixtureImages());
  process.env.THELXINOE_SERVER_IMAGE = references.server;
  process.env.THELXINOE_CONTROLLER_IMAGE = references.controller;
  if (Object.values(references).every(imageExists)) return references;
  // Images this run named are removed with its other resources; CI names them
  // for the whole workflow run, whose runners are discarded.
  if (named)
    resourceRecord(
      { images: Object.values(references), closed: false },
      process.env.THELXINOE_CI_ROOT_RESOURCES,
    );
  await stageBuildImages();
  pnpm('run', 'build:containers');
  return references;
}

async function server() {
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
  await group('rust-tests', () =>
    run('cargo', [
      'test',
      '--locked',
      '--workspace',
      '--exclude',
      'thelxinoe-desktop',
    ]),
  );
  await group('python-tests', () => node('scripts/python-tests.mjs'));
}

async function web() {
  ensurePlaywright();
  pnpm('run', 'validate:web');
  await group('ui', () =>
    pnpm('exec', 'playwright', 'test', '--config', 'playwright.ui.config.ts'),
  );
}

async function mock() {
  ensurePlaywright();
  await group('mock', () => pnpm('run', 'test:ui:mock'));
}

/** Build the images and save them for the container jobs. */
async function imagesPhase() {
  const references = await images();
  if (!githubActions) return;
  mkdirSync('.local', { recursive: true });
  run('sh', [
    '-c',
    `docker save ${references.server} ${references.controller} | gzip -1 > .local/ci-images.tar.gz`,
  ]);
}

async function services() {
  await images();
  await stageServiceImages();
  await browser();
  await group('service-access', () =>
    pnpm('run', 'test:service-access', '--built'),
  );
  await group('service-connections', () =>
    node('scripts/test-service-connections.mjs'),
  );
  await group('service-safety', () => node('scripts/test-service-safety.mjs'));
  await group('local-acceptance', () =>
    node('scripts/test-local-acceptance.mjs'),
  );
}

async function recyclarr() {
  await images();
  await stageServiceImages();
  await browser();
  await group('recyclarr', () => node('scripts/test-recyclarr.mjs'));
}

async function media() {
  const references = await images();
  await stageServiceImages();
  await browser();
  await group('server-tools', () =>
    run(process.execPath, ['scripts/test-server-tools.mjs', '--built'], {
      env: { ...process.env, THELXINOE_TOOLS_TEST_IMAGE: references.server },
    }),
  );
  await group('catalog', async () => {
    const project = fixtureId('catalog');
    node('scripts/fixtures.mjs');
    const env = await fixtureEnvironment(project);
    const catalog = composeFixture({
      project,
      file: 'compose.test.yaml',
      root: `.local/${project}`,
      env,
    });
    try {
      run('docker', [...catalog.args, 'up', '-d', '--wait'], { env });
      populateMedia(catalog.args);
      run(pnpmCommand, ['run', 'test:e2e'], {
        env: {
          ...env,
          THELXINOE_TEST_URL: `https://localhost:${env.THELXINOE_TEST_HTTPS_PORT}`,
        },
      });
      await qualifyHostRecovery(catalog, 'test-results/e2e/host-recovery.json');
    } finally {
      catalog.close();
    }
  });
  await group('playback', async () => {
    const project = fixtureId('playback');
    node('scripts/playback-fixtures.mjs');
    const env = await fixtureEnvironment(project);
    env.THELXINOE_PLAYBACK_URL = `https://localhost:${env.THELXINOE_TEST_HTTPS_PORT}`;
    const playback = composeFixture({
      project,
      file: 'compose.test.yaml',
      root: `.local/${project}`,
      env,
    });
    try {
      run('docker', [...playback.args, 'up', '-d', '--wait'], { env });
      populateMedia(playback.args);
      for (const script of [
        'test-playback.mjs',
        'test-playback-tracks.mjs',
        'test-user-media.mjs',
        'test-jellyfin.mjs',
        'test-transcoding.mjs',
      ])
        run(process.execPath, ['scripts/' + script], { env });
    } finally {
      playback.close();
    }
  });
}

async function fixtureEnvironment(project) {
  return {
    ...process.env,
    COMPOSE_PROJECT_NAME: project,
    THELXINOE_TEST_HTTP_PORT: String(await freePort({ udp: true })),
    THELXINOE_TEST_HTTPS_PORT: String(await freePort()),
    THELXINOE_TEST_OIDC_PORT: String(await freePort()),
  };
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

// Real-container ownership transfers of every supported service and storage type.
async function transfers() {
  await images();
  await stageServiceImages();
  await browser();
  await group('transfers', () => node('scripts/test-transfers.mjs'));
}

/**
 * The pinned Windows MPV and FFmpeg the native playback tests use, verified by digest.
 * Extracted copies are kept in .local/ci-tools, which CI caches by the pins.
 */
async function nativeTools() {
  const pins = JSON.parse(readFileSync('scripts/ci-native-tools.json', 'utf8'));
  const root = resolve('.local/ci-tools');
  const entries = {};
  for (const pin of pins) {
    const directory = join(root, `${pin.name}-${pin.sha256.slice(0, 12)}`);
    const entry = join(directory, pin.entry);
    if (!existsSync(entry)) {
      const response = await fetch(pin.url, {
        signal: AbortSignal.timeout(600000),
      });
      if (!response.ok)
        throw new Error(`${pin.name}: HTTP ${response.status} from ${pin.url}`);
      const bytes = Buffer.from(await response.arrayBuffer());
      const digest = createHash('sha256').update(bytes).digest('hex');
      if (bytes.length !== pin.size || digest !== pin.sha256)
        throw new Error(
          `${pin.name}: ${pin.url} no longer matches its pinned digest; pin a new build in scripts/ci-native-tools.json`,
        );
      rmSync(directory, { recursive: true, force: true });
      mkdirSync(directory, { recursive: true });
      const archive = `${directory}.download`;
      writeFileSync(archive, bytes);
      // Windows' bsdtar reads both the 7z and the zip archive.
      run(
        join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe'),
        ['-xf', archive, '-C', directory],
      );
      rmSync(archive);
    }
    entries[pin.name] = entry;
  }
  return { mpv: entries.mpv, ffmpeg: dirname(entries.ffmpeg) };
}

async function desktop() {
  if (!isWindows) throw new Error('The desktop CI phase requires Windows.');
  await group('online-storage', () =>
    run('powershell.exe', [
      '-NoProfile',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      'scripts/test-online-storage.ps1',
    ]),
  );
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
  await group('desktop-tests', () =>
    run('cargo', ['test', '--locked', '-p', 'thelxinoe-desktop']),
  );
  await group('native-playback', async () => {
    // The MPV tests drive a real player over its IPC pipe, with generated media.
    const { mpv, ffmpeg } = await nativeTools();
    const path =
      Object.keys(process.env).find((key) => key.toUpperCase() === 'PATH') ??
      'PATH';
    run(
      'cargo',
      [
        'test',
        '--locked',
        '-p',
        'thelxinoe-desktop',
        '--',
        '--ignored',
        '--exact',
        'mpv::runtime::quality_smoke::mpv_quality_reload_keeps_process_position_and_pause',
        'tools::config::runtime::tests::real_mpv_loads_one_plugin_copy_with_the_saved_settings',
      ],
      {
        env: {
          ...process.env,
          [path]: `${ffmpeg}${delimiter}${process.env[path]}`,
          THELXINOE_TEST_MPV: mpv,
          THELXINOE_MPV_TEST_EXE: mpv,
        },
      },
    );
  });
  await group('windows-process-isolation', () =>
    run('cargo', [
      'test',
      '--locked',
      '-p',
      'thelxinoe-server',
      'windows_environment_isolated_and_tree_killed',
    ]),
  );
}

// The lab serves on host loopback, which a browser on the runner reaches directly.
async function updatesServer() {
  ensurePlaywright();
  await group('release-scripts', () =>
    run(process.execPath, ['--test', 'scripts/release-checks.test.mjs']),
  );
  await group('server-updates', () =>
    node('scripts/updates/test.mjs', '--server-only'),
  );
}

// Installations update with the controller of the release they run.
async function updatesPublished() {
  await group('published-release-update', () =>
    node('scripts/updates/test-published.mjs'),
  );
}

async function updatesDesktop() {
  if (!isWindows)
    throw new Error('Desktop update qualification requires Windows.');
  ensurePlaywright();
  await group('desktop-updates', () =>
    node('scripts/updates/test.mjs', '--desktop-only'),
  );
}

const phases = {
  server,
  web,
  mock,
  images: imagesPhase,
  services,
  recyclarr,
  media,
  transfers,
  desktop,
  'updates-server': updatesServer,
  'updates-published': updatesPublished,
  'updates-desktop': updatesDesktop,
};
const windowsOnly = ['desktop', 'updates-desktop'];
const all = Object.keys(phases).filter(
  (phase) => phase !== 'images' && (isWindows || !windowsOnly.includes(phase)),
);
const requested = process.argv
  .slice(2)
  .flatMap((phase) =>
    phase === 'updates'
      ? all.filter((item) => item.startsWith('updates-'))
      : [phase],
  );
for (const phase of requested)
  if (!phases[phase]) throw new Error(`Unknown CI phase: ${phase}`);

resourceScope();
process.env.THELXINOE_CI_ROOT_RESOURCES =
  process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
for (const phase of requested.length ? requested : all) {
  console.log(`\n=== ${phase} ===`);
  try {
    await phases[phase]();
  } catch (error) {
    failures.push(phase);
    console.error(`${phase}: ${error.message}`);
  }
}
try {
  browserRuntime?.close();
} catch (error) {
  failures.push(`browser cleanup: ${error.message}`);
}
const errors = cleanupResources(
  process.env.THELXINOE_CI_ROOT_RESOURCES,
  process.env.THELXINOE_CI_RUN_ID,
  { removeImages: true },
);
if (errors.length) failures.push(...errors);
if (failures.length) {
  console.error(`\nCI failed: ${failures.join(', ')}`);
  process.exitCode = 1;
} else console.log('\nCI passed');
