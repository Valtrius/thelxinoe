import { expect } from '@playwright/test';
import {
  randomUUID,
  createCipheriv,
  createDecipheriv,
  randomBytes,
} from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { docker, fixture } from './service-access-fixture.mjs';
import { imageManifest } from './ci-images.mjs';

const output = 'test-results/service-safety';
mkdirSync(output, { recursive: true });
const result = { passed: false, scenarios: [] };
const f = await fixture({ separateMovies: true });
function controller(path, method = 'GET', body) {
  const response = f.compose(
    'exec',
    '-T',
    'controller',
    'curl',
    '-sS',
    '--unix-socket',
    '/run/thelxinoe/controller.sock',
    '-H',
    'Content-Type: application/json',
    '-X',
    method,
    '-w',
    '\n%{http_code}',
    ...(body === undefined ? [] : ['--data-binary', JSON.stringify(body)]),
    `http://localhost/stack${path}`,
  );
  const split = response.lastIndexOf('\n');
  return {
    status: Number(response.slice(split + 1)),
    body: response.slice(0, split),
  };
}
async function scenario(name, run) {
  try {
    await run();
    result.scenarios.push({ name, passed: true });
    console.log(`PASS ${name}`);
  } catch (error) {
    result.scenarios.push({ name, passed: false, error: error.message });
    throw error;
  }
}
async function adopt(attached) {
  const review = await f.api('/admin/stack/adopt/preview', 'POST', {
    service_id: attached.id,
  });
  const accepted = await f.api('/admin/stack/adopt', 'POST', {
    service_id: attached.id,
    review_id: review.review_id,
  });
  await expect
    .poll(
      async () =>
        (await f.stack()).provisions.find((row) => row.id === accepted.id)
          ?.state,
      { timeout: 90000 },
    )
    .toBe('complete');
  return { review, id: accepted.id };
}
async function release(id) {
  await f.api(`/admin/stack/${id}/action`, 'POST', { action: 'release' });
}
try {
  for (const layout of [
    'config-contains-media',
    'media-contains-config',
    'server-secondary-media',
  ]) {
    await scenario(
      `backup rejects ${layout} before stopping any container`,
      async () => {
        const directory = `${f.root}/${layout}`;
        const source =
          layout === 'server-secondary-media'
            ? `${f.root}/existing-movies`
            : directory;
        const media =
          layout === 'config-contains-media'
            ? `${source}/movies`
            : layout === 'media-contains-config'
              ? directory
              : `${f.root}/media/movies`;
        const config =
          layout === 'media-contains-config' ? `${source}/config` : source;
        mkdirSync(media, { recursive: true });
        const attached = await f.attach(
          'prowlarr',
          '/existing-prowlarr',
          imageManifest.services.prowlarr,
          {
            configDirectory: config,
            mediaMounts: [{ source: media, destination: '/movies' }],
          },
        );
        const accepted = await adopt(attached);
        expect(accepted.review.capabilities.backup.available).toBe(false);
        const before = JSON.parse(docker('inspect', attached.container))[0]
          .State.StartedAt;
        const rejected = controller('/backups', 'POST', {
          passphrase: 'service safety fixture passphrase',
        });
        expect(rejected.status).toBe(409);
        expect(
          JSON.parse(docker('inspect', attached.container))[0].State.StartedAt,
        ).toBe(before);
        expect(
          (await f.context.request.get(`${f.base}/api/v1/health`)).status(),
        ).toBe(200);
        await release(accepted.id);
      },
    );
  }
  await scenario(
    'isolated imported state restores while live media stays current',
    async () => {
      const attached = await f.attach(
        'radarr',
        '/existing-radarr',
        imageManifest.services.radarr,
        { existingLayout: true },
      );
      const accepted = await adopt(attached);
      expect(accepted.review.capabilities.backup.available).toBe(true);
      docker(
        'exec',
        '-u',
        '0',
        attached.container,
        'sh',
        '-c',
        'printf before > /config/safety-marker',
      );
      const passphrase = 'service safety fixture passphrase';
      const created = controller('/backups', 'POST', { passphrase });
      expect(created.status).toBe(200);
      const id = JSON.parse(created.body).id;
      const stage = () => {
        try {
          return JSON.parse(controller('/backups').body).items.find(
            (row) => row.id === id,
          )?.stage;
        } catch {
          return 'restarting';
        }
      };
      await expect.poll(stage, { timeout: 180000 }).toBe('complete');
      docker(
        'exec',
        '-u',
        '0',
        attached.container,
        'sh',
        '-c',
        'printf after > /config/safety-marker',
      );
      f.compose(
        'exec',
        '-T',
        '-u',
        '0',
        'server',
        'sh',
        '-c',
        'printf latest > /media/movies/safety-media',
      );
      expect(
        controller(`/backups/${id}/restore`, 'POST', { passphrase }).status,
      ).toBe(200);
      await expect.poll(stage, { timeout: 180000 }).toBe('restored');
      expect(
        docker('exec', attached.container, 'cat', '/config/safety-marker'),
      ).toBe('before');
      expect(
        f.compose('exec', '-T', 'server', 'cat', '/media/movies/safety-media'),
      ).toBe('latest');
      await expect
        .poll(
          async () => {
            try {
              return (
                await f.context.request.get(`${f.base}/api/v1/health`)
              ).status();
            } catch {
              return 0;
            }
          },
          { timeout: 90000 },
        )
        .toBe(200);
      await release(accepted.id);
    },
  );
  await scenario(
    'auto-remove adoption blocks stop, restart and backups and permits release',
    async () => {
      const attached = await f.attach(
        'radarr',
        '/existing-radarr',
        imageManifest.services.radarr,
        { existingLayout: true, autoRemove: true },
      );
      const accepted = await adopt(attached);
      expect(accepted.review.capabilities.lifecycle.available).toBe(false);
      expect(accepted.review.capabilities.backup.available).toBe(false);
      for (const action of ['stop', 'restart'])
        expect(
          controller(`/${accepted.id}/action`, 'POST', { action }).status,
        ).toBe(409);
      expect(
        controller('/backups', 'POST', {
          passphrase: 'service safety fixture passphrase',
        }).status,
      ).toBe(409);
      expect(
        JSON.parse(docker('inspect', attached.container))[0].State.Running,
      ).toBe(true);
      // Reproduce a journal left by an interrupted imported lifecycle, then let Docker --rm remove it.
      const journalPath = `/var/lib/thelxinoe/deployment/services/${accepted.id}/service.json`;
      const key = Buffer.from(
        f.compose(
          'exec',
          '-T',
          'controller',
          'base64',
          '-w0',
          '/var/lib/thelxinoe/deployment/registry-key/master.key',
        ),
        'base64',
      );
      const encrypted = Buffer.from(
        f.compose('exec', '-T', 'controller', 'base64', '-w0', journalPath),
        'base64',
      );
      const scope = Buffer.from(`service:${accepted.id}`);
      const decipher = createDecipheriv(
        'aes-256-gcm',
        key,
        encrypted.subarray(0, 12),
      );
      decipher.setAAD(scope);
      decipher.setAuthTag(encrypted.subarray(-16));
      const journal = JSON.parse(
        Buffer.concat([
          decipher.update(encrypted.subarray(12, -16)),
          decipher.final(),
        ]),
      );
      journal.phase = 'changing';
      const nonce = randomBytes(12),
        cipher = createCipheriv('aes-256-gcm', key, nonce);
      cipher.setAAD(scope);
      const changed = Buffer.concat([
        nonce,
        cipher.update(JSON.stringify(journal)),
        cipher.final(),
        cipher.getAuthTag(),
      ]);
      const file = `${f.root}/interrupted-import.json`;
      writeFileSync(file, changed);
      docker(
        'cp',
        file,
        `${f.compose('ps', '-q', 'controller')}:${journalPath}`,
      );
      f.compose('exec', '-T', 'controller', 'chmod', '600', journalPath);
      docker('stop', attached.container);
      const serverId = f.compose('ps', '-q', 'server');
      const server = JSON.parse(docker('inspect', serverId))[0];
      const normalize = (source) =>
        /^[a-zA-Z]:[\\/]/.test(source)
          ? `/run/desktop/mnt/host/${source[0].toLowerCase()}/${source.slice(3).replaceAll('\\', '/')}`
          : source;
      const backupId = randomUUID();
      const backup = {
        id: backupId,
        stage: 'quiescing',
        created_at: Math.floor(Date.now() / 1000),
        error: null,
        components: [
          {
            key: 'server',
            container: serverId,
            source: normalize(
              server.Mounts.find((m) => m.Destination === '/var/lib/thelxinoe')
                .Source,
            ),
            running: true,
          },
          {
            key: accepted.id,
            container: attached.container,
            source: normalize(
              journal.imported.storage.find((m) => m.destination === '/config')
                .source,
            ),
            running: true,
          },
        ],
        recovery: null,
        recovery_ready: false,
        rollback_phase: null,
        release_restore: null,
      };
      const backupFile = `${f.root}/interrupted-backup.json`;
      writeFileSync(backupFile, JSON.stringify(backup));
      const backupDir = `/var/lib/thelxinoe/deployment/backups/${backupId}`;
      f.compose('exec', '-T', 'controller', 'mkdir', '-p', backupDir);
      docker(
        'cp',
        backupFile,
        `${f.compose('ps', '-q', 'controller')}:${backupDir}/operation.json`,
      );
      docker('stop', serverId);
      f.compose('restart', 'controller');
      await expect
        .poll(
          async () => {
            try {
              return (
                await f.context.request.get(`${f.base}/api/v1/health`)
              ).status();
            } catch {
              return 0;
            }
          },
          { timeout: 90000 },
        )
        .toBe(200);
      const record = JSON.parse(controller('/backups').body).items.find(
        (row) => row.id === backupId,
      );
      expect(record.stage).toBe('rollback-activating');
      expect(record.error).toBeTruthy();
      await release(accepted.id);
      expect(
        controller(`/${accepted.id}/action`, 'POST', { action: 'release' })
          .status,
      ).toBe(200);
      f.compose('restart', 'controller');
      await expect
        .poll(
          () => {
            try {
              return JSON.parse(controller('/backups').body).items.find(
                (row) => row.id === backupId,
              )?.stage;
            } catch {
              return 'restarting';
            }
          },
          { timeout: 90000 },
        )
        .toBe('failed');
    },
  );
  await scenario(
    'Seerr re-resolves reused and missing remote IDs and preserves accounts across restart',
    async () => {
      await f.install('seerr');
      const me = await f.api('/auth/me');
      const email = `${me.user.id}@thelxinoe.invalid`;
      await f.api('/seerr/requests');
      const original = (
        await f.upstream('seerr', `user?q=${email}`)
      ).results.find((row) => row.email === email);
      expect(original.id).toBeGreaterThan(1);
      const replace = (sql, params) =>
        docker(
          'run',
          '--rm',
          '--network',
          'none',
          '--volumes-from',
          f.services.seerr.container_id,
          '--entrypoint',
          'python',
          docker(
            'inspect',
            '--format',
            '{{.Config.Image}}',
            f.compose('ps', '-q', 'server'),
          ),
          '-c',
          'import sqlite3,json,sys; c=sqlite3.connect("/config/db/db.sqlite3"); c.execute(sys.argv[1],json.loads(sys.argv[2])); c.commit()',
          sql,
          JSON.stringify(params),
        );
      replace('UPDATE user SET email=?,username=?,permissions=? WHERE id=?', [
        'bob@example.invalid',
        'Bob',
        32,
        original.id,
      ]);
      await f.api('/seerr/requests');
      expect(
        (await f.upstream('seerr', `user/${original.id}`)).permissions,
      ).toBe(32);
      const resolved = (
        await f.upstream('seerr', `user?q=${email}`)
      ).results.find((row) => row.email === email);
      expect(resolved.id).not.toBe(original.id);
      await f.upstream('seerr', `user/${resolved.id}`, 'DELETE');
      await f.api('/seerr/requests');
      const recovered = (
        await f.upstream('seerr', `user?q=${email}`)
      ).results.find((row) => row.email === email);
      expect(recovered.id).not.toBe(resolved.id);
      docker('restart', f.services.seerr.container_id);
      await expect
        .poll(
          async () => {
            try {
              await f.api('/seerr/requests');
              return true;
            } catch {
              return false;
            }
          },
          { timeout: 90000 },
        )
        .toBe(true);
      expect(
        (await f.upstream('seerr', `user?q=${email}`)).results.find(
          (row) => row.email === email,
        ).id,
      ).toBe(recovered.id);
    },
  );
  result.passed = true;
} finally {
  writeFileSync(`${output}/result.json`, JSON.stringify(result, null, 2));
  await f.close();
}
