import { stopProcess } from '../ci-processes.mjs';
import {
  resourceRecord,
  freePort,
  resourceScope,
  removeFixtureImages,
} from '../ci-resources.mjs';
const resourceOwners = new Map();
import { spawn } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  openSync,
  closeSync,
  appendFileSync,
  rmSync,
} from 'node:fs';
import { resolve, join, dirname, sep } from 'node:path';
import { pathToFileURL } from 'node:url';
import { tmpdir } from 'node:os';
import { request as https } from 'node:https';
import { setTimeout as delay } from 'node:timers/promises';
import { downloadFixture } from './registry.mjs';
import {
  run,
  docker,
  pullFixtureImage,
  save,
  keys,
  source,
  containerBuild,
  nativeBuild,
  envelope,
  repository,
} from './build.mjs';

export { freePort } from '../ci-resources.mjs';
export async function until(fn, timeout = 90000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    try {
      const value = await fn();
      if (value) return value;
    } catch (error) {
      if (error.fatal) throw error;
      last = error;
    }
    await delay(500);
  }
  throw last ?? Error('Update lab timed out');
}
export function publisher(lab, path = '/status', body) {
  return new Promise((done, reject) => {
    const req = https(
      lab.publisher + path,
      {
        ca: readFileSync(join(lab.root, 'tls.pem')),
        method: body ? 'POST' : 'GET',
        headers: body ? { 'Content-Type': 'application/json' } : {},
      },
      (response) => {
        let data = '';
        response.on('data', (chunk) => (data += chunk));
        response.on('end', () => {
          if (response.statusCode !== 200)
            reject(Error(`Publisher HTTP ${response.statusCode}`));
          else {
            try {
              done(JSON.parse(data));
            } catch (error) {
              reject(error);
            }
          }
        });
      },
    );
    req.on('error', reject);
    req.setTimeout(5000, () => req.destroy(Error('Publisher timeout')));
    req.end(body ? JSON.stringify(body) : undefined);
  });
}
function background(lab, name, command, args, env = {}) {
  const log = openSync(join(lab.root, `${name}.log`), 'a');
  const child = spawn(command, args, {
    cwd: repository,
    windowsHide: true,
    detached: true,
    stdio: ['ignore', log, log],
    env: { ...process.env, ...env },
  });
  child.once('spawn', () =>
    appendFileSync(
      join(lab.root, `${name}.log`),
      `Started ${name} PID ${child.pid}\n`,
    ),
  );
  child.once('error', (error) =>
    appendFileSync(
      join(lab.root, `${name}.log`),
      `Launch failed: ${error.message}\n`,
    ),
  );
  child.once('exit', (code, signal) =>
    appendFileSync(
      join(lab.root, `${name}.log`),
      `Exited: code=${code} signal=${signal}\n`,
    ),
  );
  child.unref();
  closeSync(log);
  return child.pid;
}
export function compose(lab, ...args) {
  return docker(
    'compose',
    '-p',
    lab.id,
    '-f',
    join(lab.root, 'compose.json'),
    ...args,
  );
}
export async function startPublisher(lab) {
  lab.processes.publisher = background(lab, 'publisher', process.execPath, [
    join(repository, 'scripts/updates/publisher.mjs'),
    join(lab.root, 'lab.json'),
  ]);
  save(join(lab.root, 'lab.json'), lab);
  await until(async () => (await publisher(lab)).id === lab.id);
}
export function readLab(file) {
  const path = resolve(file);
  const lab = JSON.parse(readFileSync(path, 'utf8'));
  if (
    !/^thelxinoe-update-\d+$/.test(lab.id) ||
    resolve(lab.root) !== dirname(path) ||
    !path.startsWith(resolve(repository, '.local') + sep)
  )
    throw Error('Not a local update lab descriptor');
  return lab;
}
function writeCompose(lab) {
  // Docker Desktop cannot open SQLite snapshots through deeply nested Windows binds.
  const storageRoot =
    process.platform === 'win32' ? join(tmpdir(), lab.id) : lab.root;
  lab.storage = join(storageRoot, `storage-${Date.now()}`);
  for (const directory of ['server', 'cache', 'media', 'deployment'])
    mkdirSync(join(lab.storage, directory), { recursive: true });
  docker(
    'run',
    '--rm',
    '--name',
    `${lab.id}-permissions`,
    '--label',
    `app.thelxinoe.update-lab=${lab.id}`,
    '--network',
    'none',
    '--read-only',
    '-u',
    '0:0',
    '-v',
    `${lab.storage}:/lab`,
    '--entrypoint',
    'sh',
    lab.images[lab.base].controller.reference,
    '-c',
    'chown 10001:10001 /lab/server /lab/cache /lab/media && chmod 750 /lab/server /lab/cache /lab/media',
  );
  const mount = (directory, target) =>
    `${join(lab.storage, directory).replaceAll('\\', '/')}:${target}`;
  const hardening = {
    read_only: true,
    security_opt: ['no-new-privileges:true'],
    cap_drop: ['ALL'],
  };
  save(join(lab.root, 'compose.json'), {
    name: lab.id,
    services: {
      controller: {
        ...hardening,
        image: lab.images[lab.base].controller.reference,
        container_name: `${lab.id}-controller`,
        user: '0:10001',
        network_mode: 'none',
        restart: 'unless-stopped',
        cap_add: ['CHOWN', 'FOWNER', 'DAC_OVERRIDE'],
        environment: { THELXINOE_RELEASE_KEY_FILE: '/publisher/release.pub' },
        volumes: [
          `${lab.id}-runtime:/run/thelxinoe`,
          '/var/run/docker.sock:/var/run/docker.sock',
          mount('deployment', '/var/lib/thelxinoe/deployment'),
          `${lab.root.replaceAll('\\', '/')}/trust:/publisher:ro`,
        ],
        labels: {
          'app.thelxinoe.component': 'controller',
          'app.thelxinoe.update-lab': lab.id,
        },
      },
      server: {
        ...hardening,
        image: lab.images[lab.base].server.reference,
        container_name: `${lab.id}-server`,
        user: '10001:10001',
        restart: 'unless-stopped',
        ports: [`127.0.0.1:${lab.serverPort}:8484`],
        environment: {
          THELXINOE_RELEASE_URL: `https://host.docker.internal:${lab.publisherPort}/latest.json`,
          THELXINOE_RELEASE_KEY_FILE: '/publisher/release.pub',
          THELXINOE_RELEASE_CA_FILE: '/publisher/tls.pem',
          THELXINOE_PUBLIC_URL: lab.baseUrl,
          THELXINOE_DISCOVERY: 'false',
        },
        volumes: [
          mount('server', '/var/lib/thelxinoe'),
          mount('cache', '/var/cache/thelxinoe'),
          mount('media', '/media'),
          `${lab.id}-runtime:/run/thelxinoe:ro`,
          `${lab.root.replaceAll('\\', '/')}/trust:/publisher:ro`,
        ],
        extra_hosts: ['host.docker.internal:host-gateway'],
        networks: ['media'],
        tmpfs: ['/tmp:rw,noexec,nosuid,size=64m'],
        depends_on: { controller: { condition: 'service_healthy' } },
        labels: {
          'app.thelxinoe.component': 'server',
          'app.thelxinoe.update-lab': lab.id,
        },
      },
    },
    networks: { media: {} },
    volumes: Object.fromEntries(
      ['runtime'].map((v) => [`${lab.id}-${v}`, { name: `${lab.id}-${v}` }]),
    ),
  });
}
export async function createLab({
  server = true,
  desktop = process.platform === 'win32',
  headless = false,
} = {}) {
  if (desktop && process.platform !== 'win32')
    throw Error('The native update lab requires Windows');
  const id = `thelxinoe-update-${Date.now()}${String(process.pid).padStart(10, '0')}`;
  const root = resolve(repository, '.local', id);
  mkdirSync(root, { recursive: true });
  const base = JSON.parse(
    readFileSync(join(repository, 'package.json'), 'utf8'),
  ).version;
  const parts = base.split('.').map(Number);
  parts[2]++;
  const lab = {
    id,
    root,
    base,
    next: parts.join('.'),
    server,
    desktop,
    headless,
    identifier: `app.thelxinoe.updatelab${id.split('-').at(-1)}`,
    desktopExecutable: `${id}.exe`,
    serverPort: await freePort(),
    publisherPort: await freePort(),
    registryPort: await freePort(),
    cdpPort: await freePort(),
    images: {},
    sources: {},
    processes: {},
  };
  lab.publisher = `https://localhost:${lab.publisherPort}`;
  lab.baseUrl = `http://127.0.0.1:${lab.serverPort}`;
  save(join(root, 'lab.json'), lab);
  resourceOwners.set(
    root,
    resourceRecord({
      lab: join(root, 'lab.json'),
      workspace: repository,
      closed: false,
    }),
  );
  try {
    keys(lab);
    if (server) {
      await pullFixtureImage('registry:2');
      await pullFixtureImage('node:24-bookworm-slim');
      mkdirSync(join(root, 'registry-control'), { recursive: true });
      save(join(root, 'registry-control/mode.json'), { mode: 'base' });
    }
    await startPublisher(lab);
    if (server) {
      docker(
        'network',
        'create',
        '--label',
        `app.thelxinoe.update-lab=${id}`,
        `${id}-registry`,
      );
      docker(
        'run',
        '--pull',
        'never',
        '-d',
        '--name',
        `${id}-registry`,
        '--label',
        `app.thelxinoe.update-lab=${id}`,
        '--network',
        `${id}-registry`,
        'registry:2',
      );
      docker(
        'run',
        '--pull',
        'never',
        '-d',
        '--name',
        `${id}-registry-proxy`,
        '--label',
        `app.thelxinoe.update-lab=${id}`,
        '--network',
        `${id}-registry`,
        '-p',
        `127.0.0.1:${lab.registryPort}:5000`,
        '--read-only',
        '-v',
        `${join(repository, 'scripts/updates/registry.mjs')}:/registry.mjs:ro`,
        '-v',
        `${join(root, 'registry-control')}:/control:ro`,
        'node:24-bookworm-slim',
        'node',
        '/registry.mjs',
        'serve',
        `${id}-registry`,
        '/control/mode.json',
      );
      await until(
        async () =>
          (
            await fetch(`http://127.0.0.1:${lab.registryPort}/v2/`, {
              signal: AbortSignal.timeout(5000),
            })
          ).ok,
      );
    }
    for (const version of [lab.base, lab.next]) {
      console.log(`Building update lab ${version}`);
      const directory = source(lab, version);
      lab.sources[version] = directory;
      if (server) lab.images[version] = containerBuild(lab, version, directory);
      if (server && version === lab.next) {
        lab.downloadSources = lab.images[version];
        await refreshDownloads(lab);
      }
      if (desktop) await nativeBuild(lab, version, directory);
      envelope(lab, version, lab.images[version]);
      save(join(root, 'lab.json'), lab);
    }
    if (server) writeCompose(lab);
    if (server) compose(lab, 'up', '-d', '--wait');
    else {
      for (const dir of ['state', 'cache', 'media'])
        mkdirSync(join(root, dir), { recursive: true });
      lab.processes.server = background(
        lab,
        'server',
        join(root, 'server.exe'),
        [],
        {
          THELXINOE_STATE: join(root, 'state'),
          THELXINOE_CACHE: join(root, 'cache'),
          THELXINOE_MEDIA: join(root, 'media'),
          THELXINOE_WEB: join(lab.sources[lab.base], 'frontend/dist'),
          THELXINOE_BIND: `127.0.0.1:${lab.serverPort}`,
          THELXINOE_DISCOVERY: 'false',
          THELXINOE_RELEASE_URL: `${lab.publisher}/latest.json`,
          THELXINOE_RELEASE_KEY_FILE: join(root, 'release.pub'),
          THELXINOE_RELEASE_CA_FILE: join(root, 'tls.pem'),
        },
      );
    }
    save(join(root, 'lab.json'), lab);
    await until(
      async () =>
        (
          await fetch(`${lab.baseUrl}/api/v1/health`, {
            signal: AbortSignal.timeout(5000),
          })
        ).ok,
    );
    if (desktop) installBase(lab);
    save(join(root, 'lab.json'), lab);
    return lab;
  } catch (error) {
    save(join(root, 'lab.json'), lab);
    await stopLab(lab).catch(() => {});
    throw error;
  }
}
export function installBase(lab) {
  if (!lab.desktopExecutable)
    throw Error('Start a new lab to use isolated desktop installers');
  stopDesktop(lab);
  const directory = join(lab.root, 'installed');
  mkdirSync(directory, { recursive: true });
  run(join(lab.root, 'channel', lab.base, 'setup.exe'), [
    '/S',
    `/D=${directory}`,
  ]);
  lab.processes.desktop = background(
    lab,
    'desktop',
    join(directory, lab.desktopExecutable ?? 'thelxinoe-desktop.exe'),
    [],
    {
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${lab.cdpPort}`,
      WEBVIEW2_USER_DATA_FOLDER: join(lab.root, 'webview'),
    },
  );
}
export function stopDesktop(lab) {
  if (!lab.desktop) return;
  const executable = join(
    lab.root,
    'installed',
    lab.desktopExecutable ?? 'thelxinoe-desktop.exe',
  ).replaceAll("'", "''");
  run(
    'powershell.exe',
    [
      '-NoProfile',
      '-Command',
      `$ErrorActionPreference = 'Stop'; Get-Process | Where-Object { $_.Path -eq '${executable}' } | ForEach-Object { Stop-Process -InputObject $_ -Force; if (-not $_.WaitForExit(30000)) { throw 'Update lab desktop did not exit' } }`,
    ],
    { stdio: 'ignore' },
  );
}
function removeContainers(lab, keepRegistry = false) {
  const owned = docker(
    'ps',
    '-aq',
    '--filter',
    `label=app.thelxinoe.update-lab=${lab.id}`,
  )
    .split(/\s+/)
    .filter(Boolean);
  const containers = new Set();
  const deployments = new Set();
  for (const id of owned) {
    const [container] = JSON.parse(docker('inspect', id));
    if (
      keepRegistry &&
      [`/${lab.id}-registry`, `/${lab.id}-registry-proxy`].includes(
        container.Name,
      )
    )
      continue;
    containers.add(id);
    const deployment = container.Config.Labels['app.thelxinoe.deployment'];
    if (deployment) deployments.add(deployment);
  }
  // Stop controllers before inventorying children, so they cannot create more.
  if (containers.size) docker('rm', '-f', '-v', ...containers);
  for (const deployment of deployments) {
    const children = docker(
      'ps',
      '-aq',
      '--filter',
      `label=app.thelxinoe.deployment=${deployment}`,
    )
      .split(/\s+/)
      .filter(Boolean);
    if (children.length) docker('rm', '-f', '-v', ...children);
  }
}
export async function stopLab(lab) {
  readLab(join(lab.root, 'lab.json'));
  const errors = [];
  const attempt = async (action) => {
    try {
      await action();
    } catch (error) {
      errors.push(error);
    }
  };
  await attempt(() => stopDesktop(lab));
  if (lab.desktop) await attempt(() => desktopInstallation(lab, false));
  // Publishers must be stopped even when Docker or an uninstaller fails.
  await attempt(() =>
    stopProcess(lab.processes.publisher, [
      'scripts/updates/publisher.mjs',
      join(lab.root, 'lab.json'),
    ]),
  );
  if (!lab.server)
    await attempt(() =>
      stopProcess(lab.processes.server, [join(lab.root, 'server.exe')]),
    );
  if (lab.server) {
    await attempt(() => removeContainers(lab));
    if (existsSync(join(lab.root, 'compose.json')))
      await attempt(() =>
        compose(lab, 'down', '--volumes', '--remove-orphans'),
      );
    await attempt(() => {
      const networks = docker(
        'network',
        'ls',
        '-q',
        '--filter',
        `label=app.thelxinoe.update-lab=${lab.id}`,
      )
        .split(/\s+/)
        .filter(Boolean);
      for (const network of networks)
        awaitSync(() => docker('network', 'rm', network));
    });
    // Clear every reset's UID-owned bind data before dropping its cleanup image.
    if (!errors.length && lab.storage) {
      const storageRoot =
        process.platform === 'win32'
          ? resolve(tmpdir(), lab.id)
          : resolve(lab.root);
      if (!resolve(lab.storage).startsWith(storageRoot + sep))
        errors.push(Error('Update lab storage escaped its root'));
      else if (
        existsSync(storageRoot) &&
        readdirSync(storageRoot).some((name) => /^storage-\d+$/.test(name))
      )
        await attempt(() => {
          docker(
            'run',
            '--rm',
            '--name',
            `${lab.id}-cleanup`,
            '--label',
            `app.thelxinoe.update-lab=${lab.id}`,
            '--network',
            'none',
            '--read-only',
            '--user',
            '0:0',
            '--volume',
            `${storageRoot}:/lab`,
            '--entrypoint',
            'sh',
            lab.images[lab.base].controller.reference,
            '-c',
            'find /lab -mindepth 1 -maxdepth 1 -type d -name "storage-*" -exec rm -rf -- {} +',
          );
          if (process.platform === 'win32')
            rmSync(storageRoot, {
              recursive: true,
              force: true,
              maxRetries: 5,
            });
        });
    }
    // Match this registry namespace, including downloaded digest-only candidates.
    if (!errors.length)
      await attempt(() => {
        const references = docker(
          'image',
          'ls',
          '--digests',
          '--format',
          '{{.Repository}} {{.Tag}} {{.Digest}}',
        ).split(/\r?\n/);
        const owned = new Set();
        for (const line of references) {
          const [repository, tag, digest] = line.split(' ');
          if (
            !repository?.startsWith(`localhost:${lab.registryPort}/${lab.id}/`)
          )
            continue;
          if (tag !== '<none>') owned.add(`${repository}:${tag}`);
          if (digest !== '<none>') owned.add(`${repository}@${digest}`);
        }
        removeFixtureImages(owned);
      });
  }
  function awaitSync(action) {
    try {
      action();
    } catch (error) {
      errors.push(error);
    }
  }
  if (errors.length)
    throw new AggregateError(
      errors,
      errors.map((error) => error.message).join('; '),
    );
  resourceOwners.get(lab.root)?.({ closed: true });
}
export function desktopInstallation(lab, inspect = true) {
  // Validate the descriptor before running the installer-owned removal flow.
  readLab(join(lab.root, 'lab.json'));
  return JSON.parse(
    run(
      'powershell.exe',
      [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        join(repository, 'scripts/updates/uninstall-desktop.ps1'),
        '-LabRoot',
        lab.root,
        '-ExecutableName',
        lab.desktopExecutable ?? 'thelxinoe-desktop.exe',
        ...(inspect ? ['-Inspect'] : []),
      ],
      { stdio: ['ignore', 'pipe', 'pipe'] },
    ),
  );
}
export async function resetLab(lab) {
  if (lab.desktop && !lab.desktopExecutable)
    throw Error('Start a new lab to use isolated desktop installers');
  await publisher(lab, '/control', { mode: 'base' });
  if (lab.server) {
    removeContainers(lab, true);
    compose(lab, 'down', '--volumes');
    writeCompose(lab);
    compose(lab, 'up', '-d', '--wait');
    if (lab.downloadSources) {
      await refreshDownloads(lab);
      envelope(lab, lab.next, lab.images[lab.next]);
    }
  }
  if (lab.desktop) installBase(lab);
  save(join(lab.root, 'lab.json'), lab);
}
export async function refreshDownloads(lab) {
  const images = {};
  for (const part of ['server', 'controller'])
    images[part] = await downloadFixture(
      lab,
      lab.downloadSources[part].reference,
    );
  lab.images[lab.next] = images;
}
export function serverFailure(lab, enabled) {
  if (!lab.server) throw Error('Validation failure requires the Docker lab');
  const name = `${lab.id}-server`;
  const [container] = JSON.parse(docker('inspect', name));
  if (container.Config.Labels['app.thelxinoe.update-lab'] !== lab.id)
    throw Error('Server does not belong to this lab');
  docker(
    'exec',
    name,
    'python3',
    '-c',
    `from pathlib import Path; p=Path('/var/lib/thelxinoe/fail-live-validation'); ${enabled ? "p.write_text('update lab failure')" : 'p.unlink(missing_ok=True)'}`,
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const [command = 'start', file] = process.argv.slice(2);
  if (command === 'start') {
    resourceScope({ cleanupOnExit: false });
    const lab = await createLab({
      server: !process.argv.includes('--desktop-only'),
      desktop:
        process.platform === 'win32' && !process.argv.includes('--server-only'),
    });
    console.log(
      `\nPublisher: ${lab.publisher}\nServer: ${lab.baseUrl}\nCreate admin with password: update lab passphrase\nDescriptor: ${join(lab.root, 'lab.json')}\nReset: pnpm run updates:lab reset "${join(lab.root, 'lab.json')}"\nStop: pnpm run updates:lab stop "${join(lab.root, 'lab.json')}"`,
    );
  } else {
    const lab = readLab(file);
    if (command === 'stop') await stopLab(lab);
    else if (command === 'reset') await resetLab(lab);
    else if (command === 'candidate')
      await publisher(lab, '/control', { mode: 'candidate' });
    else if (command === 'fail-validation') serverFailure(lab, true);
    else if (command === 'clear-failure') serverFailure(lab, false);
    else
      throw Error(
        'Use start, candidate, fail-validation, clear-failure, reset or stop',
      );
  }
}
