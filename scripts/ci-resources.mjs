import { execFileSync } from 'node:child_process';
import { randomBytes, randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { createSocket } from 'node:dgram';
import { existsSync, mkdirSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { jsonRead, jsonWrite } from './ci-state.mjs';

const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 120000,
  }).trim();
function inspect(kind, name) {
  try {
    return JSON.parse(docker(kind, 'inspect', name))[0];
  } catch (error) {
    if (
      /No such (?:object|container|network|image)/i.test(String(error.stderr))
    )
      return null;
    throw error;
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

export function resourceRecord(
  value,
  directory = process.env.THELXINOE_CI_RESOURCE_DIRECTORY,
) {
  if (!directory) return () => {};
  mkdirSync(directory, { recursive: true });
  const path = join(directory, randomUUID() + '.json');
  const record = { owner: process.env.THELXINOE_CI_RUN_ID, ...value };
  const update = (changes) => {
    Object.assign(record, changes);
    jsonWrite(path, record);
  };
  update({});
  return update;
}

export function composeFixture({ project, file, root, env = process.env }) {
  mkdirSync(root, { recursive: true });
  const path = join(root, 'compose.json');
  const owner = env.THELXINOE_CI_RUN_ID ?? project;
  const config = JSON.parse(
    execFileSync(
      'docker',
      ['compose', '-p', project, '-f', file, 'config', '--format', 'json'],
      { env, encoding: 'utf8', windowsHide: true },
    ),
  );
  const networks = [];
  const update = resourceRecord(
    { owner, project, compose: path, networks, closed: false },
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
      networks.push(name);
      update({ networks });
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
    for (const service of Object.values(config.services)) {
      service.labels = { ...service.labels, 'io.thelxinoe.ci-run': owner };
    }
    for (const volume of Object.values(config.volumes ?? {})) {
      volume.labels = { ...volume.labels, 'io.thelxinoe.ci-run': owner };
    }
    jsonWrite(path, config);
  } catch (error) {
    for (const name of networks) docker('network', 'rm', name);
    update({ closed: true });
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
      }).trim(),
    close() {
      execFileSync(
        'docker',
        [...args, 'down', '--volumes', '--remove-orphans'],
        { env, stdio: 'pipe', windowsHide: true, timeout: 120000 },
      );
      for (const name of networks) docker('network', 'rm', name);
      update({ closed: true });
    },
  };
}

export function cleanupResources(
  directory,
  owner,
  { removeImages = false } = {},
) {
  const errors = [];
  if (!directory || !existsSync(directory)) return errors;
  for (const file of readdirSync(directory)) {
    if (!file.endsWith('.json')) continue;
    const path = join(directory, file);
    const record = jsonRead(path);
    if (record.owner !== owner || record.closed) continue;
    if (record.images && !removeImages) continue;
    try {
      for (const id of record.containers ?? []) {
        const container = inspect('container', id);
        if (!container) continue;
        if (container.Config.Labels?.['io.thelxinoe.ci-run'] !== owner)
          throw Error('Container ownership changed');
        docker('rm', '-f', id);
      }
      if (record.deployment) {
        const ids = docker(
          'ps',
          '-aq',
          '--filter',
          `label=app.thelxinoe.deployment=${record.deployment}`,
        )
          .split(/\s+/)
          .filter(Boolean);
        if (ids.length) docker('rm', '-f', ...ids);
      }
      if (record.lab)
        execFileSync(
          process.execPath,
          ['scripts/updates/lab.mjs', 'stop', record.lab],
          {
            cwd: record.workspace,
            windowsHide: true,
            timeout: 180000,
            stdio: 'pipe',
          },
        );
      if (record.compose) {
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
      }
      for (const name of record.networks ?? []) {
        const network = inspect('network', name);
        if (!network) continue;
        if (network.Labels?.['io.thelxinoe.ci-run'] !== owner)
          throw Error('Network ownership changed');
        docker('network', 'rm', name);
      }
      if (record.images)
        for (const image of record.images) {
          if (!image.endsWith(`:${owner}`))
            throw Error('Image reference does not belong to this run');
          const current = inspect('image', image);
          if (!current) continue;
          if (record.imageIds?.[image] && record.imageIds[image] !== current.Id)
            throw Error('Image identity changed');
          docker('image', 'rm', image);
        }
      jsonWrite(path, { ...record, closed: true });
    } catch (error) {
      errors.push(`${file}: ${error.message}`);
    }
  }
  return errors;
}

export function fixtureImages() {
  const namespace = process.env.THELXINOE_CI_RUN_ID ?? randomUUID();
  return Object.fromEntries(
    ['server', 'controller'].map((component) => [
      component,
      `thelxinoe-ci-${component}:${namespace}`,
    ]),
  );
}
