// Real deployment, Linux permissions, managed updates and saved-Compose recovery.
import { expect, request } from '@playwright/test';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createServer } from 'node:net';
import { generateKeyPairSync, randomBytes, sign } from 'node:crypto';

const serverImage =
  process.env.THELXINOE_IDENTITY_SERVER_IMAGE ??
  'thelxinoe-service-server:local';
const controllerImage =
  process.env.THELXINOE_IDENTITY_CONTROLLER_IMAGE ??
  'thelxinoe-service-controller:local';
const docker = (...args) =>
  execFileSync('docker', args, {
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'pipe'],
    maxBuffer: 8 * 1024 * 1024,
  }).trim();
const inspect = (id) => JSON.parse(docker('inspect', id))[0];
const evidence = [];
mkdirSync('.local', { recursive: true });
if (!process.argv.includes('--built')) {
  for (const [target, image] of [
    ['server', serverImage],
    ['controller', controllerImage],
  ])
    execFileSync('docker', ['build', '--target', target, '-t', image, '.'], {
      stdio: 'inherit',
    });
}

async function freePort() {
  const listener = createServer();
  await new Promise((done, reject) => {
    listener.once('error', reject);
    listener.listen(0, '127.0.0.1', done);
  });
  const port = listener.address().port;
  await new Promise((done) => listener.close(done));
  return port;
}

async function scenario(uid, gid, full) {
  const project = `thelxinoe-identity-${Date.now()}`;
  const linuxRoot = `/tmp/${project}`;
  const root = resolve(`.local/${project}`);
  mkdirSync(root, { recursive: true });
  const identity = `${uid}:${gid}`;
  const port = await freePort();
  const base = `http://localhost:${port}`;
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  writeFileSync(
    `${root}/release.pub`,
    publicKey
      .export({ type: 'spki', format: 'der' })
      .subarray(-32)
      .toString('base64'),
  );
  const config = JSON.parse(
    execFileSync(
      'docker',
      [
        'compose',
        '--env-file',
        '.env.example',
        '-p',
        project,
        '-f',
        'compose.yaml',
        'config',
        '--format',
        'json',
      ],
      {
        encoding: 'utf8',
        env: {
          ...process.env,
          THELXINOE_UID: full ? String(uid) : '',
          THELXINOE_GID: full ? String(gid) : '',
          THELXINOE_SERVER_IMAGE: serverImage,
          THELXINOE_CONTROLLER_IMAGE: controllerImage,
          THELXINOE_PUBLIC_URL: '',
          THELXINOE_CORS_ORIGINS: '',
          THELXINOE_TRUSTED_PROXIES: '',
        },
      },
    ),
  );
  const volumes = {
    '/var/lib/thelxinoe': 'state',
    '/var/cache/thelxinoe': 'cache',
    '/media': 'media',
    '/backups': 'backups',
    '/var/lib/thelxinoe/deployment': 'deployment',
  };
  for (const [name, service] of Object.entries(config.services)) {
    delete service.build;
    service.container_name = `${project}-${name}`;
    service.restart = 'no';
    for (const mount of service.volumes) {
      if (volumes[mount.target]) {
        mount.type = 'bind';
        mount.source = `${linuxRoot}/${volumes[mount.target]}`;
      }
    }
    service.volumes.push({
      type: 'bind',
      source: `${root}/release.pub`,
      target: '/etc/thelxinoe/release.pub',
      read_only: true,
    });
  }
  config.services.server.ports = [
    {
      target: 8484,
      published: String(port),
      host_ip: '127.0.0.1',
      protocol: 'tcp',
    },
  ];
  config.services.server.environment.THELXINOE_DISCOVERY = 'false';
  config.services.server.environment.THELXINOE_RELEASE_URL = '';
  const composeFile = `${root}/compose.json`;
  writeFileSync(composeFile, JSON.stringify(config, null, 2));
  const compose = (...args) =>
    docker('compose', '-p', project, '-f', composeFile, ...args);
  const client = await request.newContext({
    baseURL: `${base}/api/v1/`,
    extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
  });
  const api = async (path, method = 'GET', data) => {
    const response = await client.fetch(path, { method, data });
    if (!response.ok())
      throw Error(`${path}: ${response.status()} ${await response.text()}`);
    return response.json();
  };
  const controller = (path, data) =>
    JSON.parse(
      execFileSync(
        'docker',
        [
          'exec',
          '-i',
          `${project}-controller`,
          'curl',
          '-fsS',
          '--unix-socket',
          '/run/thelxinoe/controller.sock',
          ...(data
            ? ['-H', 'Content-Type: application/json', '--data-binary', '@-']
            : []),
          `http://localhost${path}`,
        ],
        { encoding: 'utf8', input: data ? JSON.stringify(data) : undefined },
      ),
    );
  const result = { uid, gid, checks: [], passed: false };
  evidence.push(result);
  let deployment;
  const external = [];
  function checked(name) {
    result.checks.push(name);
    console.log(`${identity}: ${name}`);
  }
  async function readyService(id) {
    let stack;
    await expect
      .poll(
        async () => {
          stack = await api('admin/stack');
          const provision = stack.provisions.find((p) => p.id === id);
          if (provision?.state === 'blocked') throw Error(provision.error);
          return provision?.state;
        },
        { timeout: 300000, intervals: [1500] },
      )
      .toBe('complete');
    return stack.items.find((s) => s.id === id);
  }
  async function update(service) {
    // A stopped service gives an unambiguous idle state for this permissions check.
    await api(`admin/stack/${service.id}/action`, 'POST', { action: 'stop' });
    const update = await api(
      `admin/service-updates/preflight/${service.id}`,
      'POST',
      {},
    );
    const wait = async (expected) => {
      await expect
        .poll(
          async () => {
            const current = (await api('admin/service-updates')).items.find(
              (u) => u.id === update.id,
            );
            if (
              [
                'blocked',
                'runtime-failure',
                'rolled-back',
                'recovery-required',
              ].includes(current?.state)
            )
              throw Error(current.error);
            return current?.state;
          },
          { timeout: 360000, intervals: [1500] },
        )
        .toBe(expected);
    };
    await wait('ready');
    await api(`admin/service-updates/${update.id}/activate`, 'POST', {});
    await wait('committed');
    await api(`admin/stack/${service.id}/action`, 'POST', { action: 'start' });
    return (await api('admin/stack')).items.find((s) => s.id === service.id);
  }
  try {
    // Daemon-side Linux binds enforce Unix permissions under Docker Desktop.
    docker(
      'run',
      '--rm',
      '--user',
      '0:0',
      '--entrypoint',
      'sh',
      '-v',
      `${linuxRoot}:/seed`,
      controllerImage,
      '-c',
      `mkdir -p /seed/state /seed/cache /seed/media /seed/backups /seed/deployment /seed/external && chown ${identity} /seed/state /seed/cache /seed/media /seed/backups && chmod 770 /seed/state /seed/cache /seed/media /seed/backups`,
    );
    compose('up', '-d', '--wait', '--wait-timeout', '180');
    await api('setup', 'POST', {
      username: 'admin',
      password: 'test-only long passphrase',
    });
    deployment = (await api('admin/stack')).deployment_id;
    expect(docker('exec', `${project}-server`, 'id', '-u')).toBe(String(uid));
    expect(docker('exec', `${project}-server`, 'id', '-g')).toBe(String(gid));
    expect(
      docker(
        'exec',
        `${project}-controller`,
        'stat',
        '-c',
        '%g',
        '/run/thelxinoe/controller.sock',
      ),
    ).toBe(String(gid));
    docker(
      'exec',
      `${project}-server`,
      'sh',
      '-c',
      'touch /media/server-write /var/cache/thelxinoe/cache-write /backups/backup-write',
    );
    checked('server writes and private controller socket work');
    compose('restart', 'controller');
    await expect
      .poll(async () => {
        try {
          return (await api('admin/stack')).deployment_id;
        } catch {
          return '';
        }
      })
      .toBe(deployment);
    checked('controller restart preserves socket access');
    if (full) {
      for (const kind of ['radarr', 'seerr']) {
        const created = await api('admin/stack/install', 'POST', {
          kind,
          host_port: await freePort(),
        });
        const service = await readyService(created.id);
        const raw = inspect(service.container_id);
        if (kind === 'seerr') expect(raw.Config.User).toBe(identity);
        else {
          expect(raw.Config.Env).toContain(`PUID=${uid}`);
          expect(raw.Config.Env).toContain(`PGID=${gid}`);
          docker(
            'exec',
            '--user',
            identity,
            service.container_id,
            'sh',
            '-c',
            'printf shared > /media/shared-write',
          );
          expect(
            docker('exec', `${project}-server`, 'cat', '/media/shared-write'),
          ).toBe('shared');
        }
        checked(`${kind} installation and file access`);
        const updated = await update(service);
        expect(updated.container_id).not.toBe(service.container_id);
        const appdata = inspect(updated.container_id).Mounts.find(
          (m) => m.Destination === '/config',
        ).Source;
        docker(
          'run',
          '--rm',
          '--user',
          identity,
          '--entrypoint',
          'sh',
          '-v',
          `${appdata}:/config`,
          controllerImage,
          '-c',
          'touch /config/identity-sentinel',
        );
        docker('rm', '-f', updated.container_id);
        await api(`admin/stack/${service.id}/action`, 'POST', {
          action: 'recreate',
        });
        const recovered = (await api('admin/stack')).items.find(
          (s) => s.id === service.id,
        );
        docker(
          'exec',
          '--user',
          identity,
          recovered.container_id,
          'sh',
          '-c',
          'test -f /config/identity-sentinel && touch /config/after-recovery',
        );
        checked(`${kind} preflight, update and missing-container recovery`);
      }

      // Adoption preserves a different UID and an omitted PGID (image default 911).
      const template = controller('/stack/templates').items.find(
        (t) => t.kind === 'prowlarr',
      );
      const key = randomBytes(16).toString('hex');
      execFileSync(
        'docker',
        [
          'run',
          '--rm',
          '--user',
          '0:0',
          '--entrypoint',
          'sh',
          '-i',
          '-v',
          `${linuxRoot}/external:/config`,
          controllerImage,
          '-c',
          'cat > /config/config.xml',
        ],
        {
          input: `<Config><BindAddress>*</BindAddress><Port>9696</Port><EnableSsl>False</EnableSsl><LaunchBrowser>False</LaunchBrowser><ApiKey>${key}</ApiKey><AuthenticationMethod>External</AuthenticationMethod><AuthenticationRequired>DisabledForLocalAddresses</AuthenticationRequired><UpdateAutomatically>False</UpdateAutomatically></Config>`,
        },
      );
      const original = docker(
        'run',
        '-d',
        '--name',
        `${project}-external-prowlarr`,
        '--network',
        `${project}_default`,
        '-e',
        'PUID=34567',
        '-e',
        'TZ=UTC',
        '-v',
        `${linuxRoot}/external:/config`,
        `${template.repository}@${template.digest}`,
      );
      external.push(original);
      await expect
        .poll(
          () => {
            try {
              return docker(
                'exec',
                original,
                'curl',
                '-fsS',
                'http://127.0.0.1:9696/ping',
              );
            } catch {
              return '';
            }
          },
          { timeout: 90000, intervals: [1500] },
        )
        .toContain('OK');
      const attached = await api('admin/support', 'POST', {
        name: 'Existing account fixture',
        kind: 'prowlarr',
        container_id: original,
        port: 9696,
        credentials: { username: '', secret: key },
      });
      const originalDeployment = inspect(original);
      const review = await api('admin/stack/adopt/preview', 'POST', {
        service_id: attached.id,
      });
      const adopted = await api('admin/stack/adopt', 'POST', {
        service_id: attached.id,
        review_id: review.review_id,
      });
      const service = await readyService(adopted.id);
      expect(service.container_id).toBe(original);
      expect(inspect(original).Config).toEqual(originalDeployment.Config);
      expect(inspect(original).State.StartedAt).toBe(
        originalDeployment.State.StartedAt,
      );
      docker(
        'exec',
        '--user',
        '34567:911',
        service.container_id,
        'chmod',
        '600',
        '/config/config.xml',
      );
      const preflight = await client.post(
        `admin/service-updates/preflight/${service.id}`,
        { data: {} },
      );
      expect(preflight.status()).toBe(409);
      const env = inspect(service.container_id).Config.Env;
      expect(env).toContain('PUID=34567');
      expect(env.some((entry) => entry.startsWith('PGID='))).toBe(false);
      docker(
        'exec',
        '--user',
        '34567:911',
        service.container_id,
        'sh',
        '-c',
        'touch /config/adopted-write',
      );
      checked(
        'in-place adoption preserves UID and the default GID; unsupported recreation remains blocked',
      );

      // Exercise product snapshot verification and server validation with these IDs.
      // The signed newer version deliberately disagrees with the old executable;
      // rejection must happen after both workers have successfully written reports.
      const image = (name) => {
        const raw = JSON.parse(docker('image', 'inspect', name))[0];
        const reference = raw.RepoDigests[0];
        return {
          reference: reference.split('@')[0].includes('/')
            ? reference
            : `docker.io/library/${reference}`,
        };
      };
      const schema = Number(
        readFileSync('crates/database/src/lib.rs', 'utf8').match(
          /SCHEMA_VERSION: u32 = (\d+)/,
        )[1],
      );
      const currentVersion = JSON.parse(readFileSync('package.json', 'utf8'))
        .version.split('.')
        .map(Number);
      currentVersion[2]++;
      const now = Math.floor(Date.now() / 1000);
      const payload = Buffer.from(
        JSON.stringify({
          format: 1,
          version: currentVersion.join('.'),
          published_at: now - 60,
          expires_at: now + 3600,
          server: image(serverImage),
          controller: image(controllerImage),
          windows_x64: {
            url: 'https://example.invalid/fixture.exe',
            sha256: 'a'.repeat(64),
            bytes: 1,
            signature: 'fixture',
            updater_public_key: 'fixture',
          },
          api: { min: 1, max: 1 },
          migration: {
            from: { min: schema, max: schema },
            target: schema,
            recovery: 'full-state-restore',
            recovery_protocol: 1,
          },
          notes: '',
        }),
      );
      const product = controller('/stack/product/preflight', {
        envelope: {
          payload: payload.toString('base64'),
          signature: sign(
            null,
            Buffer.concat([
              Buffer.from('Thelxinoe release manifest v1\0'),
              payload,
            ]),
            privateKey,
          ).toString('base64'),
        },
      });
      let outcome;
      await expect
        .poll(
          () => {
            outcome = controller('/stack/product').items.find(
              (u) => u.id === product.id,
            );
            return outcome.stage;
          },
          { timeout: 180000, intervals: [1000] },
        )
        .toBe('blocked');
      expect(outcome.error).toBe(
        'Server migration contract does not match the signed release',
      );
      const reportPath = `/var/lib/thelxinoe/deployment/product-updates/${product.id}/clone/.release-validation.json`;
      expect(
        docker(
          'exec',
          `${project}-controller`,
          'stat',
          '-c',
          '%u:%g',
          reportPath,
        ),
      ).toBe(identity);
      checked('product workers validate copied state using configured IDs');
    }
    // Use the real accepted deployment export to recreate the server and controller.
    for (const name of ['compose.yaml', 'compose.override.yaml'])
      writeFileSync(
        `${root}/${name}`,
        docker(
          'exec',
          `${project}-controller`,
          'cat',
          `/var/lib/thelxinoe/deployment/${name}`,
        ),
      );
    docker(
      'compose',
      '-p',
      project,
      '-f',
      `${root}/compose.yaml`,
      '-f',
      `${root}/compose.override.yaml`,
      'up',
      '-d',
      '--force-recreate',
      '--wait',
      '--wait-timeout',
      '180',
    );
    expect(docker('exec', `${project}-server`, 'id', '-u')).toBe(String(uid));
    expect(docker('exec', `${project}-server`, 'id', '-g')).toBe(String(gid));
    expect((await api('admin/stack')).deployment_id).toBe(deployment);
    checked('accepted Compose recreation preserves IDs and data');
    result.passed = true;
  } catch (error) {
    result.error = error.message;
    for (const name of ['server', 'controller']) {
      try {
        const logs = spawnSync(
          'docker',
          ['logs', '--tail', '40', `${project}-${name}`],
          { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
        );
        writeFileSync(
          `${root}/${name}.log`,
          `${logs.stdout ?? ''}${logs.stderr ?? ''}`,
        );
      } catch {
        /* The container may not have been created. */
      }
    }
    throw error;
  } finally {
    await client.dispose();
    if (external.length) docker('rm', '-f', ...external);
    if (deployment) {
      const owned = docker(
        'ps',
        '-aq',
        '--filter',
        `label=app.thelxinoe.deployment=${deployment}`,
      )
        .split(/\s+/)
        .filter(Boolean);
      if (owned.length) docker('rm', '-f', ...owned);
    }
    compose('down', '-v');
    docker(
      'run',
      '--rm',
      '--user',
      '0:0',
      '--entrypoint',
      'sh',
      '-v',
      `${linuxRoot}:/fixture`,
      controllerImage,
      '-c',
      'rm -rf /fixture/state /fixture/cache /fixture/media /fixture/backups /fixture/deployment /fixture/external',
    );
    writeFileSync(
      '.local/container-identity-result.json',
      JSON.stringify(evidence, null, 2),
    );
  }
}

await scenario(10001, 10001, false);
await scenario(12345, 23456, true);
