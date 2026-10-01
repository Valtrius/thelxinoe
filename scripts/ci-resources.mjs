import { execFileSync } from 'node:child_process';
import { randomBytes, randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { createSocket } from 'node:dgram';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { resolve, join, sep, basename } from 'node:path';
import { jsonRead, jsonWrite } from './ci-state.mjs';
import { fixtureImage, imageManifest } from './ci-images.mjs';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: 'pipe',
    windowsHide: true,
    timeout: 120000,
  }).trim();
function inspect(kind, name) {
  try {
    return JSON.parse(docker(kind, 'inspect', name))[0];
  } catch (error) {
    if (
      /No such (?:object|container|network|image|volume)/i.test(
        String(error.stderr),
      )
    )
      return null;
    throw error;
  }
}
export function removeFixtureImages(references) {
  const images = new Set();
  const remove = (reference) => {
    try {
      docker('image', 'rm', reference);
    } catch (error) {
      // Another completed run can remove a shared worker alias concurrently.
      if (inspect('image', reference)) throw error;
    }
  };
  for (const reference of references) {
    const current = inspect('image', reference);
    if (!current) continue;
    images.add(current.Id);
    remove(reference);
  }
  for (const id of images) {
    const current = inspect('image', id);
    if (!current) continue;
    const repositories = new Set([
      'thelxinoe-controller-worker',
      'thelxinoe-recovery-controller',
      'thelxinoe-recovery-server',
    ]);
    const aliases = new Set(
      [...repositories].map((repo) => `${repo}:${id.slice(7)}`),
    );
    // A concurrent run can share a cached build. Its image references or
    // containers must survive until that run performs its own cleanup.
    if (
      current.RepoTags?.some((tag) => !aliases.has(tag)) ||
      current.RepoDigests?.some(
        (digest) => !repositories.has(digest.split('@')[0]),
      ) ||
      docker('ps', '-aq', '--filter', `ancestor=${id}`)
    )
      continue;
    for (const reference of [
      ...(current.RepoTags ?? []),
      ...(current.RepoDigests ?? []),
    ])
      remove(reference);
  }
}
// Leave room for service suffixes within Docker DNS's 63-byte label limit.
export const fixtureId = (purpose) =>
  `thelxinoe-${purpose}-${randomBytes(12).toString('hex')}`;
const allocated = new Set();
export async function freePort({ udp = false } = {}) {
  for (let attempt = 0; attempt < 20; attempt++) {
    const tcp = createServer();
    const datagram = udp ? createSocket('udp4') : null;
    try {
      await new Promise((done, reject) => {
        tcp.once('error', reject);
        tcp.listen(0, '127.0.0.1', done);
      });
      const port = tcp.address().port;
      if (allocated.has(port)) continue;
      if (datagram)
        await new Promise((done, reject) => {
          datagram.once('error', reject);
          datagram.bind(port, '127.0.0.1', done);
        });
      allocated.add(port);
      return port;
    } catch (error) {
      if (error.code !== 'EADDRINUSE') throw error;
    } finally {
      if (tcp.listening) await new Promise((done) => tcp.close(done));
      if (datagram) {
        try {
          datagram.close();
        } catch {
          /* Bind failed. */
        }
      }
    }
  }
  throw Error('Could not allocate a free fixture port');
}

let standalone;
export function resourceScope({ cleanupOnExit = true } = {}) {
  if (process.env.THELXINOE_CI_RESOURCE_DIRECTORY) return;
  const owner = randomUUID();
  const directory = resolve('.local/test-runs', owner);
  process.env.THELXINOE_CI_RUN_ID = owner;
  process.env.THELXINOE_CI_RESOURCE_DIRECTORY = directory;
  standalone = { owner, directory };
  if (!cleanupOnExit) return;
  process.once('exit', () => {
    const errors = cleanupResources(standalone.directory, standalone.owner, {
      removeImages: true,
    });
    if (errors.length) {
      console.error('Fixture cleanup failed:', errors.join('\n'));
      process.exitCode = 1;
    }
  });
}

export function resourceRecord(value, directory) {
  if (!directory) {
    resourceScope();
    directory = process.env.THELXINOE_CI_RESOURCE_DIRECTORY;
  }
  mkdirSync(directory, { recursive: true });
  const path = join(directory, randomUUID() + '.json');
  const record = { owner: process.env.THELXINOE_CI_RUN_ID, ...value };
  const update = (changes) => {
    Object.assign(record, changes);
    jsonWrite(path, record);
  };
  update({});
  update.close = () => {
    const errors = cleanupRecord(record);
    if (errors.length)
      throw new AggregateError(
        errors,
        errors.map((error) => error.message).join('; '),
      );
    update({ closed: true });
  };
  return update;
}

export function composeFixture({ project, file, root, env = process.env }) {
  resourceScope();
  env = {
    ...env,
    THELXINOE_CI_RUN_ID: process.env.THELXINOE_CI_RUN_ID,
    THELXINOE_CI_RESOURCE_DIRECTORY:
      process.env.THELXINOE_CI_RESOURCE_DIRECTORY,
  };
  mkdirSync(root, { recursive: true });
  const path = resolve(root, 'compose.json');
  const bindRoot = resolve(root);
  if (
    !bindRoot.startsWith(resolve('.local') + sep) ||
    basename(bindRoot) !== project
  )
    throw Error('Fixture data must have its own directory under .local');
  const owner = env.THELXINOE_CI_RUN_ID;
  const config = JSON.parse(
    execFileSync(
      'docker',
      ['compose', '-p', project, '-f', file, 'config', '--format', 'json'],
      { env, encoding: 'utf8', windowsHide: true, timeout: 30000 },
    ),
  );
  const networks = Object.keys(config.networks ?? {}).map(
    (key) => `${project}_${key}`,
  );
  const update = resourceRecord(
    { owner, project, networks, closed: false },
    env.THELXINOE_CI_RESOURCE_DIRECTORY,
  );
  try {
    for (const key of Object.keys(config.networks ?? {})) {
      const name = `${project}_${key}`;
      docker(
        'network',
        'create',
        '--label',
        `io.thelxinoe.ci-run=${owner}`,
        name,
      );
      const [network] = JSON.parse(docker('network', 'inspect', name));
      const subnet = network.IPAM.Config.find(
        (entry) => entry.Subnet && !entry.Subnet.includes(':'),
      )?.Subnet;
      if (!subnet) throw Error(`Missing IPv4 subnet for ${name}`);
      config.networks[key] = { external: true, name };
      for (const service of Object.values(config.services)) {
        if (
          Object.hasOwn(service.networks ?? {}, key) &&
          service.environment?.THELXINOE_TRUSTED_PROXIES
        )
          service.environment.THELXINOE_TRUSTED_PROXIES = subnet;
      }
    }
    for (const [name, service] of Object.entries(config.services)) {
      service.image = fixtureImage(service.image);
      service.labels = { ...service.labels, 'io.thelxinoe.ci-run': owner };
      if (name === 'controller')
        service.environment = {
          ...service.environment,
          THELXINOE_CURATED_IMAGES: JSON.stringify(imageManifest.services),
        };
    }
    for (const volume of Object.values(config.volumes ?? {}))
      volume.labels = { ...volume.labels, 'io.thelxinoe.ci-run': owner };
    jsonWrite(path, config);
    update({
      compose: path,
      bindRoot,
      cleanupImage:
        config.services.controller?.image ?? config.services.server?.image,
    });
  } catch (error) {
    try {
      update.close();
    } catch (cleanup) {
      throw new AggregateError(
        [error, cleanup],
        'Fixture setup and cleanup failed',
        { cause: cleanup },
      );
    }
    throw error;
  }
  const args = ['compose', '-p', project, '-f', path];
  return {
    args,
    config,
    update,
    compose: (...command) =>
      execFileSync('docker', [...args, ...command], {
        env,
        encoding: 'utf8',
        windowsHide: true,
        timeout: 360000,
      }).trim(),
    close: () => update.close(),
  };
}

function cleanupRecord(record) {
  const errors = [];
  const attempt = (action) => {
    try {
      return action();
    } catch (error) {
      errors.push(error);
    }
  };
  const owner = record.owner;
  const deployments = new Set(record.deployment ? [record.deployment] : []);
  if (record.lab)
    attempt(() =>
      execFileSync(
        process.execPath,
        ['scripts/updates/lab.mjs', 'stop', record.lab],
        {
          cwd: record.workspace,
          windowsHide: true,
          timeout: 300000,
          stdio: 'pipe',
        },
      ),
    );
  // Stop producers first, including a controller whose setup failed before its
  // deployment ID could be recorded. Then remove its managed service children.
  if (record.project) {
    const ids =
      attempt(() =>
        docker(
          'ps',
          '-aq',
          '--filter',
          `label=com.docker.compose.project=${record.project}`,
        ),
      ) ?? '';
    for (const id of ids.split(/\s+/).filter(Boolean))
      attempt(() => {
        const container = inspect('container', id);
        if (!container) return;
        if (container.Config.Labels?.['io.thelxinoe.ci-run'] !== owner)
          throw Error('Container ownership changed');
        const deployment =
          container.Config.Labels?.['app.thelxinoe.deployment'];
        if (deployment) deployments.add(deployment);
        docker('rm', '-f', '-v', id);
      });
  }
  for (const id of [
    ...(record.containers ?? []),
    ...(record.project ? [`${record.project}-cleanup`] : []),
  ])
    attempt(() => {
      const container = inspect('container', id);
      if (!container) return;
      if (container.Config.Labels?.['io.thelxinoe.ci-run'] !== owner)
        throw Error('Container ownership changed');
      docker('rm', '-f', '-v', id);
    });
  for (const deployment of deployments) {
    const ids =
      attempt(() =>
        docker(
          'ps',
          '-aq',
          '--filter',
          `label=app.thelxinoe.deployment=${deployment}`,
        ),
      ) ?? '';
    for (const id of ids.split(/\s+/).filter(Boolean))
      attempt(() => docker('rm', '-f', '-v', id));
  }
  if (record.compose)
    attempt(() => {
      const config = jsonRead(record.compose);
      if (
        Object.values(config.services).some(
          (service) => service.labels?.['io.thelxinoe.ci-run'] !== owner,
        )
      )
        throw Error('Fixture ownership changed');
      docker(
        'compose',
        '-p',
        record.project,
        '-f',
        record.compose,
        'down',
        '--volumes',
        '--remove-orphans',
      );
    });
  for (const name of record.networks ?? [])
    attempt(() => {
      const network = inspect('network', name);
      if (!network) return;
      if (network.Labels?.['io.thelxinoe.ci-run'] !== owner)
        throw Error('Network ownership changed');
      docker('network', 'rm', name);
    });
  for (const name of record.volumes ?? [])
    attempt(() => {
      const volume = inspect('volume', name);
      if (!volume) return;
      if (volume.Labels?.['io.thelxinoe.ci-run'] !== owner)
        throw Error('Volume ownership changed');
      docker('volume', 'rm', name);
    });
  if (record.bindRoot && record.cleanupImage && !errors.length)
    attempt(() => {
      const root = resolve(record.bindRoot);
      if (
        basename(root) !== record.project ||
        !root.startsWith(resolve('.local') + sep)
      )
        throw Error('Fixture data escaped its workspace');
      // Linux fixtures write as UID 10001. Delete only directories under this
      // fixture's validated root; root-level reports, traces and logs survive.
      docker(
        'run',
        '--rm',
        '--name',
        `${record.project}-cleanup`,
        '--label',
        `io.thelxinoe.ci-run=${owner}`,
        '--network',
        'none',
        '--read-only',
        '--user',
        '0:0',
        '--volume',
        `${root}:/fixture`,
        '--entrypoint',
        'sh',
        record.cleanupImage,
        '-c',
        'find /fixture -mindepth 1 -maxdepth 1 -type d -exec rm -rf -- {} +',
      );
    });
  for (const image of record.images ?? [])
    attempt(() => {
      if (!image.endsWith(`:${owner}`))
        throw Error('Image reference does not belong to this run');
      const current = inspect('image', image);
      if (!current) return;
      if (record.imageIds?.[image] && record.imageIds[image] !== current.Id)
        throw Error('Image identity changed');
      removeFixtureImages([image]);
    });
  return errors;
}

export function cleanupResources(
  directory,
  owner,
  { removeImages = false } = {},
) {
  const errors = [];
  if (!directory || !existsSync(directory)) return errors;
  const records = [];
  function collect(root) {
    for (const entry of readdirSync(root, { withFileTypes: true })) {
      const file = join(root, entry.name);
      if (entry.isDirectory()) collect(file);
      else if (entry.isFile() && entry.name.endsWith('.json')) {
        try {
          records.push({ file, record: jsonRead(file) });
        } catch (error) {
          errors.push(`${file}: ${error.message}`);
        }
      }
    }
  }
  collect(directory);
  // Image records must outlive every container that uses them.
  records.sort((a, b) => Number(!!a.record.images) - Number(!!b.record.images));
  for (const { file, record } of records) {
    if (
      record.owner !== owner ||
      record.closed ||
      record.process ||
      (record.images && (!removeImages || errors.length))
    )
      continue;
    const failures = cleanupRecord(record);
    errors.push(...failures.map((error) => `${file}: ${error.message}`));
    if (!failures.length) jsonWrite(file, { ...record, closed: true });
  }
  return errors;
}

export function fixtureImages() {
  resourceScope();
  const namespace = process.env.THELXINOE_CI_RUN_ID;
  return Object.fromEntries(
    ['server', 'controller'].map((component) => [
      component,
      `thelxinoe-ci-${component}:${namespace}`,
    ]),
  );
}
