import { waitForState, budgets } from '../ci-readiness.mjs';
import { fixtureImage, stageBuildImages } from '../ci-images.mjs';
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
  copyFileSync,
  writeFileSync,
  rmSync,
} from 'node:fs';
import { resolve, join, dirname, sep, posix } from 'node:path';
import { pathToFileURL } from 'node:url';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { request as https } from 'node:https';
import { downloadFixture, tagFixture } from './registry.mjs';
import { checkSums, feed, verifyEnvelope } from '../release-checks.mjs';
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
export function until(fn, timeout = budgets.restart) {
  return waitForState('Update lab readiness', fn, (value) => !!value, {
    timeout,
    interval: 500,
  });
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
const deployment = (lab) => join(lab.root, 'deployment');
/** The `latest` reference the lab's Compose file follows, as deployments do. */
export const latestImage = (lab, part) =>
  `localhost:${lab.registryPort}/${lab.id}/${part}:latest`;
/** Docker Compose in the lab deployment directory, configured by its .env alone. */
export function compose(lab, ...args) {
  return run(
    'docker',
    ['compose', '--project-directory', deployment(lab), '-p', lab.id, ...args],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: Object.fromEntries(
        Object.entries(process.env).filter(
          ([key]) => !/^(THELXINOE|COMPOSE)_/i.test(key),
        ),
      ),
    },
  ).trim();
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
// The shipped compose.release.yaml with a host-style .env. A separate override
// adds only what the lab needs: unique names and port, its publisher and trust.
async function writeDeployment(lab) {
  const storageDirectory = `storage-${Date.now()}`;
  // Use the same disk-backed Linux storage on every runner. Docker Desktop's
  // Windows bridge is slow for tool snapshots and its /tmp is memory-backed.
  lab.storageVolume = `${lab.id}-storage`;
  save(join(lab.root, 'lab.json'), lab);
  docker(
    'volume',
    'create',
    '--label',
    `app.thelxinoe.update-lab=${lab.id}`,
    lab.storageVolume,
  );
  const volume = storageVolume(lab);
  if (!volume) throw Error('Update lab storage volume is missing');
  lab.storage = posix.join(volume.Mountpoint, storageDirectory);
  save(join(lab.root, 'lab.json'), lab);
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
    `${lab.storageVolume}:/lab`,
    '--entrypoint',
    'sh',
    lab.images[lab.base].controller.reference,
    '-c',
    'root="/lab/$1"; mkdir -p "$root/server" "$root/cache" "$root/media" "$root/backups" && chown 10001:10001 "$root/server" "$root/cache" "$root/media" "$root/backups" && chmod 750 "$root/server" "$root/cache" "$root/media" "$root/backups"',
    '--',
    storageDirectory,
  );
  const directory = deployment(lab);
  mkdirSync(directory, { recursive: true });
  copyFileSync(lab.composeFile, join(directory, 'compose.yaml'));
  const storage = lab.storage.replaceAll('\\', '/');
  writeFileSync(
    join(directory, '.env'),
    Object.entries({
      THELXINOE_UID: 10001,
      THELXINOE_GID: 10001,
      THELXINOE_STATE_ROOT: storage,
      THELXINOE_MEDIA_ROOT: `${storage}/media`,
      THELXINOE_BACKUP_ROOT: `${storage}/backups`,
      THELXINOE_LISTEN: '127.0.0.1',
      THELXINOE_PUBLIC_URL: lab.baseUrl,
      THELXINOE_DISCOVERY: false,
      THELXINOE_SERVER_IMAGE: latestImage(lab, 'server'),
      THELXINOE_CONTROLLER_IMAGE: latestImage(lab, 'controller'),
    })
      .map(([key, value]) => `${key}=${value}\n`)
      .join(''),
  );
  // JSON values are YAML; the port list replaces the shipped one.
  const yaml = JSON.stringify;
  const trust = [`${lab.root.replaceAll('\\', '/')}/trust:/publisher:ro`];
  const labels = { 'app.thelxinoe.update-lab': lab.id };
  writeFileSync(
    join(directory, 'compose.override.yaml'),
    `services:
  controller:
    container_name: ${yaml(`${lab.id}-controller`)}
    environment: ${yaml({ THELXINOE_RELEASE_KEY_FILE: '/publisher/release.pub' })}
    volumes: ${yaml(trust)}
    labels: ${yaml(labels)}
  server:
    container_name: ${yaml(`${lab.id}-server`)}
    ports: !override ${yaml([`127.0.0.1:${lab.serverPort}:8484`])}
    environment: ${yaml({
      THELXINOE_TOOLS_CATALOG: '/opt/thelxinoe/tools/catalog.json',
      THELXINOE_RELEASE_URL: `https://host.docker.internal:${lab.publisherPort}/latest.json`,
      THELXINOE_RELEASE_KEY_FILE: '/publisher/release.pub',
      THELXINOE_RELEASE_CA_FILE: '/publisher/tls.pem',
    })}
    volumes: ${yaml(trust)}
    extra_hosts: ["host.docker.internal:host-gateway"]
    labels: ${yaml(labels)}
`,
  );
  // `latest` names the base release, locally and in the lab registry.
  for (const part of ['server', 'controller']) {
    const reference = lab.images[lab.base][part].reference;
    await tagFixture(lab, reference, 'latest');
    docker('tag', reference, latestImage(lab, part));
  }
}
function storageVolume(lab) {
  if (lab.storageVolume !== `${lab.id}-storage`)
    throw Error('Update lab storage volume escaped its owner');
  let volume;
  try {
    [volume] = JSON.parse(docker('volume', 'inspect', lab.storageVolume));
  } catch (error) {
    if (/No such volume/i.test(String(error.stderr))) return null;
    throw error;
  }
  if (volume.Labels?.['app.thelxinoe.update-lab'] !== lab.id)
    throw Error('Update lab storage volume ownership changed');
  return volume;
}
// GitHub and GHCR are remote; ride out brief outages before failing.
async function retried(action) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await action();
    } catch (error) {
      if (attempt === 4) throw error;
      console.log(`Retrying after: ${error.message}`);
      await new Promise((done) => setTimeout(done, 5000 * attempt));
    }
  }
}
/** The manifest of the live signed feed. Releases exist, so a missing feed fails. */
export async function publishedRelease() {
  const response = await retried(async () => {
    const response = await fetch(feed, { signal: AbortSignal.timeout(60000) });
    if (!response.ok) throw Error(`Release feed HTTP ${response.status}`);
    return response;
  });
  return verifyEnvelope(
    await response.json(),
    readFileSync(join(repository, 'releases/release.pub'), 'utf8'),
  );
}
// The published release's own Compose file and anonymously pulled images,
// copied into the lab registry so its `latest` tags stay the lab's own.
async function importPublished(lab, manifest) {
  const directory = join(lab.root, 'published');
  const download = (name) =>
    retried(async () => {
      const response = await fetch(
        `https://github.com/Valtrius/thelxinoe/releases/download/${manifest.version}/${name}`,
        { signal: AbortSignal.timeout(60000) },
      );
      if (!response.ok) throw Error(`Release ${name} HTTP ${response.status}`);
      return Buffer.from(await response.arrayBuffer());
    });
  mkdirSync(join(directory, 'docker'), { recursive: true });
  const file = await download('compose.yaml');
  const sums = (await download('SHA256SUMS')).toString('utf8');
  checkSums(
    sums
      .split(/\r?\n/)
      .filter((line) => line.endsWith('  compose.yaml'))
      .join('\n'),
    { 'compose.yaml': createHash('sha256').update(file).digest('hex') },
  );
  lab.composeFile = join(directory, 'compose.yaml');
  writeFileSync(lab.composeFile, file);
  const images = {};
  for (const part of ['server', 'controller']) {
    const source = manifest[part].reference;
    // An empty Docker configuration pulls as a host without credentials.
    // A stalled pull is killed and retried instead of holding the lane.
    await retried(() =>
      run('docker', ['pull', '--platform', 'linux/amd64', source], {
        env: { ...process.env, DOCKER_CONFIG: join(directory, 'docker') },
        timeout: 600000,
      }),
    );
    const tag = `localhost:${lab.registryPort}/${lab.id}/${part}:${lab.base}`;
    docker('tag', source, tag);
    run('docker', ['push', tag]);
    const [image] = JSON.parse(docker('image', 'inspect', tag));
    images[part] = {
      reference: image.RepoDigests.find((v) =>
        v.startsWith(tag.split(':').slice(0, -1).join(':') + '@'),
      ),
    };
  }
  return images;
}
export async function createLab({
  server = true,
  desktop = process.platform === 'win32',
  headless = false,
  published = null,
} = {}) {
  if (desktop && process.platform !== 'win32')
    throw Error('The native update lab requires Windows');
  if (published && (desktop || !server))
    throw Error('A published release base qualifies the Docker server only');
  const id = `thelxinoe-update-${Date.now()}${String(process.pid).padStart(10, '0')}`;
  const root = resolve(repository, '.local', id);
  mkdirSync(root, { recursive: true });
  const current = JSON.parse(
    readFileSync(join(repository, 'package.json'), 'utf8'),
  ).version;
  const parts = current.split('.').map(Number);
  parts[2]++;
  const lab = {
    id,
    root,
    base: published?.version ?? current,
    published: published?.version ?? null,
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
      await stageBuildImages();
      await pullFixtureImage(fixtureImage('registry:2'));
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
        fixtureImage('registry:2'),
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
        fixtureImage('node:24-bookworm-slim'),
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
      if (published && version === lab.base) {
        console.log(`Importing published release ${version}`);
        lab.images[version] = await importPublished(lab, published);
        envelope(lab, version, lab.images[version]);
        save(join(root, 'lab.json'), lab);
        continue;
      }
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
    lab.composeFile ??= join(lab.sources[lab.base], 'compose.release.yaml');
    if (server) await writeDeployment(lab);
    if (server)
      compose(
        lab,
        'up',
        '-d',
        '--wait',
        '--wait-timeout',
        String(budgets.startup / 1000),
      );
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
    if (server) {
      try {
        save(join(root, 'startup-state.json'), {
          server: JSON.parse(
            docker('inspect', '--format', '{{json .State}}', `${id}-server`),
          ),
          controller: JSON.parse(
            docker(
              'inspect',
              '--format',
              '{{json .State}}',
              `${id}-controller`,
            ),
          ),
          logs: compose(lab, 'logs', '--no-color'),
        });
      } catch {
        /* A build failure may occur before the containers exist. */
      }
    }
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
function inspectContainer(id) {
  try {
    return JSON.parse(docker('inspect', id))[0];
  } catch (error) {
    if (/No such (?:object|container)/i.test(String(error.stderr))) return null;
    throw error;
  }
}
function removeContainerIds(ids) {
  if (!ids.length) return;
  try {
    docker('rm', '-f', '-v', ...ids);
  } catch (error) {
    // A running controller can finish and remove a worker after inventory.
    if (ids.some((id) => inspectContainer(id))) throw error;
  }
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
  const storageRoots = new Set();
  if (lab.storageVolume) {
    const volume = storageVolume(lab);
    if (volume) {
      const prefix = volume.Mountpoint + '/';
      if (
        !lab.storage?.startsWith(prefix) ||
        !/^storage-\d+$/.test(lab.storage.slice(prefix.length))
      )
        throw Error('Update lab storage escaped its volume');
      storageRoots.add(volume.Mountpoint);
    }
  }
  for (const id of owned) {
    const container = inspectContainer(id);
    if (!container) continue;
    if (
      keepRegistry &&
      [`/${lab.id}-registry`, `/${lab.id}-registry-proxy`].includes(
        container.Name,
      )
    )
      continue;
    containers.add(id);
    for (const mount of container.Mounts ?? [])
      if (
        ['/var/lib/thelxinoe', '/var/lib/thelxinoe/deployment'].includes(
          mount.Destination,
        )
      )
        storageRoots.add(posix.dirname(mount.Source));
    const deployment = container.Config.Labels['app.thelxinoe.deployment'];
    if (deployment) deployments.add(deployment);
  }
  // Stop controllers before inventorying children, so they cannot create more.
  removeContainerIds([...containers]);
  // Bootstrap containers do not carry the deployment label. Workers do, and
  // their mounts prove that they belong to this lab's private storage root.
  const workers = docker(
    'ps',
    '-aq',
    '--filter',
    'label=app.thelxinoe.deployment',
  )
    .split(/\s+/)
    .filter(Boolean);
  for (const id of workers) {
    const container = inspectContainer(id);
    if (!container) continue;
    if (
      container.Mounts?.some((mount) =>
        [...storageRoots].some((root) => mount.Source.startsWith(root + '/')),
      )
    )
      deployments.add(container.Config.Labels['app.thelxinoe.deployment']);
  }
  for (const deployment of deployments) {
    const children = docker(
      'ps',
      '-aq',
      '--filter',
      `label=app.thelxinoe.deployment=${deployment}`,
    )
      .split(/\s+/)
      .filter(Boolean);
    removeContainerIds(children);
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
    if (existsSync(join(deployment(lab), 'compose.yaml')))
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
    if (!errors.length && lab.storageVolume) {
      await attempt(() => {
        if (storageVolume(lab)) docker('volume', 'rm', lab.storageVolume);
      });
    } else if (!errors.length && lab.storage) {
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
    await writeDeployment(lab);
    compose(
      lab,
      'up',
      '-d',
      '--wait',
      '--wait-timeout',
      String(budgets.startup / 1000),
    );
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
