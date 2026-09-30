import { spawn, execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  copyFileSync,
  openSync,
  closeSync,
  unlinkSync,
  readdirSync,
} from 'node:fs';
import { dirname, join, resolve, relative, isAbsolute, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { requestedPhases, ciPhases } from './ci-phases.mjs';
import { reporter } from './ci-report.mjs';
import {
  acquireRun,
  alive,
  atomicWrite,
  jsonRead,
  jsonWrite,
  originIdentity,
  requestRun,
} from './ci-state.mjs';
import { cleanupResources, freePort, resourceRecord } from './ci-resources.mjs';

const repository = resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
if (['--stop', '--recover'].includes(args[0])) {
  const recoveryDirectory = resolve(args[1]);
  const previous = jsonRead(join(recoveryDirectory, 'result.json'));
  if (args[0] === '--stop') {
    const active = await requestRun(previous.origin);
    if (active.id !== previous.id)
      throw Error('This run no longer owns its worktree');
    await requestRun(previous.origin, 'stop');
  } else if (!previous.finished) {
    if (alive(previous.pid)) throw Error('The coordinator is still alive');
    const release = await acquireRun(previous.origin, {
      id: previous.id,
      directory: recoveryDirectory,
      pid: process.pid,
    });
    try {
      terminateChildren(previous);
      const errors = cleanupResources(previous.resources, previous.id);
      failPending(
        previous,
        'The local CI coordinator exited without finishing this run.',
      );
      previous.cleanup_errors = errors;
      const view = reporter(recoveryDirectory, previous);
      view.save();
    } finally {
      await release();
    }
  }
  process.exit(0);
}
const noOpen = args.includes('--no-open');
if (noOpen) args.splice(args.indexOf('--no-open'), 1);
const origin = originIdentity(repository);
const id = new Date().toISOString().replace(/[:.]/g, '-') + '-' + randomUUID();
const output = args.indexOf('--output');
const directory =
  output < 0
    ? join(repository, '.local/ci', id)
    : resolve(args.splice(output, 2)[1]);
const phases = requestedPhases(args);
mkdirSync(directory, { recursive: true });
closeSync(openSync(join(directory, 'run.lock'), 'wx'));
const report = {
  id,
  pid: process.pid,
  directory,
  origin,
  resources: join(directory, 'resources'),
  ready: false,
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
mkdirSync(report.resources, { recursive: true });
const environment = {
  ...process.env,
  CI: 'true',
  THELXINOE_CI_RUN_ID: id,
  THELXINOE_CI_RESOURCE_DIRECTORY: report.resources,
  PLAYWRIGHT_BROWSERS_PATH: join(repository, '.local/ci-browsers'),
};
const children = new Set();
let refresh, browsers, release, stopping;

function terminateChildren(run) {
  if (!existsSync(run.resources)) return;
  for (const file of readdirSync(run.resources)) {
    if (!file.endsWith('.json')) continue;
    const record = jsonRead(join(run.resources, file));
    if (
      !record.process ||
      record.closed ||
      record.owner !== run.id ||
      !alive(record.process)
    )
      continue;
    const commandLine =
      process.platform === 'win32'
        ? execFileSync(
            'powershell.exe',
            [
              '-NoProfile',
              '-Command',
              `(Get-CimInstance Win32_Process -Filter "ProcessId = ${Number(record.process)}").CommandLine`,
            ],
            { encoding: 'utf8', windowsHide: true },
          )
        : execFileSync('ps', ['-p', String(record.process), '-o', 'args='], {
            encoding: 'utf8',
          });
    if (!commandLine.includes(run.id) || !commandLine.includes('ci-child.mjs'))
      throw Error('Refusing to terminate a process whose ownership changed');
    if (process.platform === 'win32')
      execFileSync(
        'taskkill.exe',
        ['/PID', String(record.process), '/T', '/F'],
        { windowsHide: true, stdio: 'pipe' },
      );
    else process.kill(-record.process, 'SIGKILL');
  }
}
function failPending(run, error) {
  run.finished = new Date().toISOString();
  run.passed = false;
  run.error = error;
  for (const lane of run.lanes) {
    if (lane.finished) continue;
    lane.state = lane.started ? 'failed' : 'cancelled';
    lane.finished = run.finished;
    lane.exit_code = 1;
    lane.error = error;
    const path = lane.workspace && join(lane.workspace, '.local/ci-steps.json');
    if (path && existsSync(path)) {
      const steps = jsonRead(path);
      for (const step of steps)
        if (!step.finished)
          Object.assign(step, { finished: run.finished, passed: false, error });
      jsonWrite(path, steps);
    }
  }
}
function stop() {
  if (stopping) return;
  stopping = true;
  report.error = 'Local CI was stopped.';
  try {
    terminateChildren(report);
  } catch (error) {
    report.error += ` ${error.message}`;
  }
}
process.on('SIGINT', stop);
process.on('SIGTERM', stop);

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

function command(command, argv, cwd, log, env = environment) {
  if (stopping) throw Error('Local CI was stopped.');
  return new Promise((done, reject) => {
    const descriptor = openSync(log, 'a');
    writeFileSync(descriptor, `\n> ${[command, ...argv].join(' ')}\n`);
    const child = spawn(
      process.execPath,
      [join(repository, 'scripts/ci-child.mjs'), id, cwd, command, ...argv],
      {
        cwd,
        env,
        windowsHide: true,
        detached: process.platform !== 'win32',
        stdio: ['ignore', descriptor, descriptor],
      },
    );
    closeSync(descriptor);
    children.add(child);
    const record = resourceRecord(
      { owner: id, process: child.pid, closed: false },
      report.resources,
    );
    child.once('close', () => {
      children.delete(child);
      record({ closed: true });
    });
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
function snapshot() {
  const files = [
    ...new Set(
      execFileSync(
        'git',
        ['ls-files', '-z', '--cached', '--others', '--exclude-standard'],
        { cwd: repository, encoding: 'utf8' },
      )
        .split('\0')
        .filter(
          (file) =>
            file && !resolve(repository, file).startsWith(directory + sep),
        ),
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
    origin,
    revision: origin.revision,
    sha256: hash.digest('hex'),
    files: files.filter((file) => !missing.includes(file)),
    deleted: missing,
  };
  if (originIdentity(repository).revision !== origin.revision)
    throw Error('HEAD changed during source capture. Launch CI again.');
  atomicWrite(
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
    const laneEnvironment = {
      ...environment,
      CARGO_TARGET_DIR: join(repository, '.local/ci-targets', phase),
    };
    if (phase === 'web') {
      laneEnvironment.THELXINOE_LAYOUT_PORT = String(await freePort());
      laneEnvironment.THELXINOE_PLAYER_PORT = String(await freePort());
    }
    item.ports = Object.fromEntries(
      Object.entries(laneEnvironment).filter(
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
      laneEnvironment,
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
  release = await acquireRun(origin, { id, directory, pid: process.pid }, stop);
  mkdirSync(join(repository, '.local/ci'), { recursive: true });
  atomicWrite(
    join(repository, '.local/ci/latest.json'),
    JSON.stringify(
      { id: report.id, directory, started: report.started },
      null,
      2,
    ) + '\n',
  );
  const source = snapshot();
  report.ready = true;
  view.save();
  await view.style(repository).catch(() => {});
  jsonWrite(join(directory, 'ready.json'), {
    id,
    directory,
    origin,
    pid: process.pid,
  });
  if (!noOpen) openReport();
  refresh = setInterval(() => {
    try {
      view.save();
    } catch (error) {
      console.error('Could not refresh the CI report:', error.message);
    }
  }, 5000);
  await Promise.all(report.lanes.map((item) => lane(item, source)));
  report.passed =
    !stopping && report.lanes.every((item) => item.state === 'passed');
} catch (error) {
  report.error = String(error.message ?? error);
  report.passed = false;
} finally {
  clearInterval(refresh);
  if (stopping || report.error) {
    while (children.size) await new Promise((done) => setTimeout(done, 50));
    report.cleanup_errors = cleanupResources(report.resources, id);
    failPending(report, report.error);
  } else {
    report.cleanup_errors = cleanupResources(report.resources, id);
    if (report.cleanup_errors.length) report.passed = false;
  }
  report.finished = new Date().toISOString();
  view.save();
  if (release) await release();
  console.log(readFileSync(join(directory, 'summary.txt'), 'utf8'));
  console.log(`Summary: ${join(directory, 'index.html')}`);
  process.exitCode = report.passed ? 0 : 1;
}
