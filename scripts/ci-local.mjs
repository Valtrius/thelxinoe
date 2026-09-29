import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  copyFileSync,
  openSync,
  closeSync,
  unlinkSync,
} from 'node:fs';
import { dirname, join, resolve, relative, isAbsolute, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { requestedPhases, ciPhases } from './ci-phases.mjs';
import { reporter } from './ci-report.mjs';

const repository = resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const output = args.indexOf('--output');
const directory =
  output < 0
    ? join(
        repository,
        '.local/ci',
        new Date().toISOString().replace(/[:.]/g, '-'),
      )
    : resolve(args.splice(output, 2)[1]);
const phases = requestedPhases(args);
mkdirSync(directory, { recursive: true });
const report = {
  id: directory.split(/[\\/]/).at(-1),
  started: new Date().toISOString(),
  finished: null,
  passed: null,
  source: null,
  lanes: phases.map((phase) => ({
    phase,
    state: 'queued',
    started: null,
    tests_started: null,
    finished: null,
    exit_code: null,
  })),
};
const view = reporter(directory, report);
view.save();
const lock = createServer((socket) => socket.end());
let refresh, browsers;

function openReport() {
  const path = join(directory, 'index.html');
  const log = join(directory, 'browser-open.log');
  const descriptor = openSync(log, 'a');
  writeFileSync(descriptor, `Opening ${path} at ${new Date().toISOString()}\n`);
  const child =
    process.platform === 'win32'
      ? spawn(
          'powershell.exe',
          [
            '-NoProfile',
            '-STA',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            join(repository, 'scripts/ci-open-report.ps1'),
            '-ReportPath',
            path,
          ],
          {
            windowsHide: true,
            // Detached Windows PowerShell can exit without executing -File.
            detached: false,
            stdio: ['ignore', descriptor, descriptor],
          },
        )
      : spawn(
          process.platform === 'darwin' ? 'open' : 'xdg-open',
          [
            ...(process.platform === 'darwin' ? ['-g'] : []),
            pathToFileURL(path).href,
          ],
          { detached: true, stdio: ['ignore', descriptor, descriptor] },
        );
  closeSync(descriptor);
  child.once('error', (error) => {
    writeFileSync(log, `Launch failed: ${error.message}\n`, { flag: 'a' });
    console.error(`Could not open the CI report: ${error.message}`);
  });
  child.once('exit', (code) => {
    writeFileSync(log, `Launcher exited with code ${code}\n`, { flag: 'a' });
    if (code !== 0) console.error(`Could not open the CI report. See ${log}`);
  });
  child.unref();
}

function command(command, argv, cwd, log, env = process.env) {
  return new Promise((done, reject) => {
    const descriptor = openSync(log, 'a');
    writeFileSync(descriptor, `\n> ${[command, ...argv].join(' ')}\n`);
    const child = spawn(command, argv, {
      cwd,
      env,
      windowsHide: true,
      stdio: ['ignore', descriptor, descriptor],
    });
    closeSync(descriptor);
    child.once('error', reject);
    child.once('exit', (code, signal) =>
      code === 0
        ? done()
        : reject(
            Error(
              `${command} exited ${signal ?? code ?? 'without a result'}. See ${log}`,
            ),
          ),
    );
  });
}
function inside(root, file) {
  const path = resolve(root, file);
  const suffix = relative(root, path);
  if (
    !suffix ||
    isAbsolute(suffix) ||
    suffix === '..' ||
    suffix.startsWith('..' + sep)
  )
    throw Error('Source path escaped its workspace');
  return path;
}
async function port() {
  const listener = createServer();
  await new Promise((done, reject) => {
    listener.once('error', reject);
    listener.listen(0, '127.0.0.1', done);
  });
  const value = listener.address().port;
  await new Promise((done) => listener.close(done));
  return String(value);
}
function snapshot() {
  const files = [
    ...new Set(
      execFileSync(
        'git',
        ['ls-files', '-z', '--cached', '--others', '--exclude-standard'],
        { cwd: repository, encoding: 'utf8' },
      )
        .split('\0')
        .filter(Boolean),
    ),
  ].sort();
  const root = join(directory, 'source');
  mkdirSync(root, { recursive: true });
  const hash = createHash('sha256');
  const missing = [];
  for (const file of files) {
    const original = inside(repository, file);
    hash.update(file + '\0');
    if (!existsSync(original)) {
      missing.push(file);
      hash.update('deleted\0');
      continue;
    }
    const bytes = readFileSync(original);
    hash.update(bytes);
    hash.update('\0');
    const destination = inside(root, file);
    mkdirSync(dirname(destination), { recursive: true });
    writeFileSync(destination, bytes);
  }
  const source = {
    revision: execFileSync('git', ['rev-parse', 'HEAD'], {
      cwd: repository,
      encoding: 'utf8',
    }).trim(),
    sha256: hash.digest('hex'),
    files: files.filter((file) => !missing.includes(file)),
    deleted: missing,
  };
  writeFileSync(
    join(directory, 'source.json'),
    JSON.stringify(source, null, 2) + '\n',
  );
  report.source = { revision: source.revision, sha256: source.sha256 };
  return { root, ...source };
}
async function lane(item, source) {
  const { phase } = item;
  const workspace = join(directory, 'workspaces', phase);
  const phaseDirectory = join(directory, 'phases', phase);
  mkdirSync(phaseDirectory, { recursive: true });
  const log = join(phaseDirectory, 'output.log');
  item.started = new Date().toISOString();
  item.state = 'preparing';
  item.workspace = workspace;
  item.log = log;
  console.log(`[${phase}] preparing isolated workspace`);
  view.save();
  try {
    await command(
      'git',
      ['worktree', 'add', '--detach', workspace, source.revision],
      repository,
      log,
    );
    for (const file of source.files) {
      const destination = inside(workspace, file);
      mkdirSync(dirname(destination), { recursive: true });
      copyFileSync(inside(source.root, file), destination);
    }
    for (const file of source.deleted) {
      const path = inside(workspace, file);
      if (existsSync(path)) unlinkSync(path);
    }
    if (process.platform === 'win32') {
      await command(
        process.env.ComSpec ?? 'cmd.exe',
        ['/d', '/s', '/c', 'npm ci'],
        workspace,
        log,
      );
    } else await command('npm', ['ci'], workspace, log);
    await view
      .style(workspace)
      .catch((error) => console.error(`Report styling: ${error.message}`));
    if (
      ['web', 'containers', 'updates-server', 'updates-desktop'].includes(phase)
    ) {
      browsers ??= command(
        process.execPath,
        [
          join(workspace, 'node_modules/playwright/cli.js'),
          'install',
          'chromium',
        ],
        workspace,
        join(directory, 'browser-install.log'),
      );
      await browsers;
    }
    item.tests_started = new Date().toISOString();
    const environment = {
      ...process.env,
      CI: 'true',
      CARGO_TARGET_DIR: join(repository, '.local/ci-targets', phase),
    };
    if (phase === 'web') {
      environment.THELXINOE_LAYOUT_PORT = await port();
      environment.THELXINOE_PLAYER_PORT = await port();
    }
    if (phase === 'containers') {
      environment.COMPOSE_PROJECT_NAME = `thelxinoe-ci-${report.id.toLowerCase().replace(/[^a-z0-9]/g, '')}`;
      for (const name of [
        'THELXINOE_TEST_HTTP_PORT',
        'THELXINOE_TEST_HTTPS_PORT',
        'THELXINOE_PLAYBACK_HTTP_PORT',
        'THELXINOE_PLAYBACK_HTTPS_PORT',
      ])
        environment[name] = await port();
    }
    item.ports = Object.fromEntries(
      Object.entries(environment).filter(
        ([key]) => key.endsWith('_PORT') && key.startsWith('THELXINOE_'),
      ),
    );
    item.state = 'running';
    console.log(`[${phase}] running shared CI phase`);
    view.save();
    await command(
      process.execPath,
      [join(workspace, 'scripts/ci.mjs'), phase],
      workspace,
      log,
      environment,
    );
    item.state = 'passed';
    item.exit_code = 0;
  } catch (error) {
    item.state = 'failed';
    item.exit_code = 1;
    item.error = String(error.message ?? error);
  } finally {
    item.finished = new Date().toISOString();
    view.save();
    console.log(`[${phase}] ${item.state.toUpperCase()} · ${log}`);
  }
}

try {
  for (const phase of phases)
    if (!ciPhases().includes(phase))
      throw Error(`Unsupported local CI phase: ${phase}`);
  await new Promise((done, reject) => {
    lock.once('error', () =>
      reject(
        Error(
          'Another local CI run owns the fixed browser and Docker fixtures. Wait for its completion window.',
        ),
      ),
    );
    lock.listen(
      process.platform === 'win32'
        ? '\\\\.\\pipe\\thelxinoe-local-ci'
        : join(tmpdir(), 'thelxinoe-local-ci.sock'),
      done,
    );
  });
  mkdirSync(join(repository, '.local/ci'), { recursive: true });
  writeFileSync(
    join(repository, '.local/ci/latest.json'),
    JSON.stringify(
      { id: report.id, directory, started: report.started },
      null,
      2,
    ) + '\n',
  );
  const source = snapshot();
  view.save();
  openReport();
  refresh = setInterval(view.save, 5000);
  await Promise.all(report.lanes.map((item) => lane(item, source)));
  report.passed = report.lanes.every((item) => item.state === 'passed');
} catch (error) {
  report.error = String(error.message ?? error);
  report.passed = false;
} finally {
  clearInterval(refresh);
  if (lock.listening) await new Promise((done) => lock.close(done));
  report.finished = new Date().toISOString();
  view.save();
  console.log(readFileSync(join(directory, 'summary.txt'), 'utf8'));
  console.log(`Summary: ${join(directory, 'index.html')}`);
  process.exitCode = report.passed ? 0 : 1;
}
