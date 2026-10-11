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
const seerrOnly = process.argv.includes('--seerr-only');
mkdirSync(output, { recursive: true });
const result = { passed: false, scenarios: [] };
const f = await fixture({
  separateMovies: true,
  moviesDirectory: 'shared/movies',
});
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
  if (seerrOnly && !name.startsWith('Seerr')) return;
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
function backupStage(id) {
  try {
    const row = JSON.parse(controller('/backups').body).items.find(
      (row) => row.id === id,
    );
    // Failure stages carry the controller's reason into the assertion message.
    return row?.error && /failed|required/.test(row.stage)
      ? `${row.stage}: ${row.error}`
      : row?.stage;
  } catch {
    return 'restarting';
  }
}
async function healthy() {
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
}
const startedAt = (container) =>
  JSON.parse(docker('inspect', container))[0].State.StartedAt;
function updateJournal(id, change) {
  const path = `/var/lib/thelxinoe/deployment/services/${id}/service.json`;
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
    f.compose('exec', '-T', 'controller', 'base64', '-w0', path),
    'base64',
  );
  const scope = Buffer.from(`service:${id}`);
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
  change(journal);
  const nonce = randomBytes(12),
    cipher = createCipheriv('aes-256-gcm', key, nonce);
  cipher.setAAD(scope);
  const changed = Buffer.concat([
    nonce,
    cipher.update(JSON.stringify(journal)),
    cipher.final(),
    cipher.getAuthTag(),
  ]);
  const file = `${f.root}/changed-${id}.json`;
  writeFileSync(file, changed);
  docker('cp', file, `${f.compose('ps', '-q', 'controller')}:${path}`);
  f.compose('exec', '-T', 'controller', 'chmod', '600', path);
  return journal;
}
try {
  for (const layout of [
    'config-contains-media',
    'media-contains-config',
    'server-secondary-media',
  ]) {
    await scenario(
      `backup leaves ${layout} running and out of the archive`,
      async () => {
        const directory = `${f.root}/${layout}`;
        const source =
          layout === 'server-secondary-media' ? `${f.root}/shared` : directory;
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
          'radarr',
          '/existing-radarr',
          imageManifest.services.radarr,
          {
            configDirectory: config,
            mediaMounts: [{ source: media, destination: '/movies' }],
          },
        );
        const accepted = await adopt(attached);
        expect(accepted.review.capabilities.backup.available).toBe(false);
        // Old reviews can have a stale positive capability; live preflight must still reject the layout.
        updateJournal(accepted.id, (journal) => {
          journal.imported.capabilities.backup.available = true;
        });
        expect(
          (await f.stack()).items.find((row) => row.id === accepted.id)
            .capabilities.backup.available,
        ).toBe(false);
        const before = startedAt(attached.container);
        const created = controller('/backups', 'POST', {
          passphrase: 'service safety fixture passphrase',
        });
        expect(created.status).toBe(200);
        const backup = JSON.parse(created.body);
        expect(backup.excluded.map((row) => row.key)).toEqual([accepted.id]);
        expect(backup.components.map((row) => row.key)).not.toContain(
          accepted.id,
        );
        await expect
          .poll(() => backupStage(backup.id), { timeout: 180000 })
          .toBe('complete');
        expect(startedAt(attached.container)).toBe(before);
        await healthy();
        await release(accepted.id);
      },
    );
  }
  await scenario(
    'isolated imported state restores only when chosen while live media stays current',
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
      expect(
        JSON.parse(created.body).components.find(
          (row) => row.key === accepted.id,
        ),
      ).toMatchObject({ kind: 'radarr', imported: true });
      const stage = () => backupStage(id);
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
      // Without a choice, the adopted service keeps running with its live configuration.
      const running = startedAt(attached.container);
      expect(
        controller(`/backups/${id}/restore`, 'POST', { passphrase }).status,
      ).toBe(200);
      await expect.poll(stage, { timeout: 180000 }).toBe('restored');
      expect(
        docker('exec', attached.container, 'cat', '/config/safety-marker'),
      ).toBe('after');
      expect(startedAt(attached.container)).toBe(running);
      await healthy();
      expect(
        controller(`/backups/${id}/restore`, 'POST', {
          passphrase,
          components: [accepted.id],
        }).status,
      ).toBe(200);
      await expect.poll(stage, { timeout: 180000 }).toBe('restored');
      expect(
        docker('exec', attached.container, 'cat', '/config/safety-marker'),
      ).toBe('before');
      expect(
        f.compose('exec', '-T', 'server', 'cat', '/media/movies/safety-media'),
      ).toBe('latest');
      // The staged swap leaves no workspace behind in the service configuration.
      expect(
        docker('exec', attached.container, 'ls', '-A', '/config'),
      ).not.toContain('.thelxinoe-restore');
      // Decrypted archives and recovery captures are plaintext; only the journal stays.
      expect(
        f.compose(
          'exec',
          '-T',
          'controller',
          'ls',
          '-A',
          `/var/lib/thelxinoe/deployment/backups/${id}`,
        ),
      ).toBe('operation.json');
      await healthy();
      await release(accepted.id);
    },
  );
  await scenario(
    'a reviewed configuration change resumes lifecycle actions on an adopted service',
    async () => {
      const attached = await f.attach(
        'radarr',
        '/existing-radarr',
        imageManifest.services.radarr,
        { existingLayout: true },
      );
      const accepted = await adopt(attached);
      // The host changes the container outside Thelxinoe, as a NAS UI would.
      docker('update', '--restart', 'always', attached.container);
      await expect
        .poll(
          async () =>
            (await f.stack()).items.find((row) => row.id === accepted.id)
              ?.drift,
        )
        .toBe(true);
      expect(
        controller(`/${accepted.id}/action`, 'POST', { action: 'stop' }).status,
      ).toBe(409);
      const review = await f.api(`/admin/stack/${accepted.id}/drift`);
      expect(review.blocked).toBeNull();
      expect(review.changes).toContainEqual(
        expect.objectContaining({
          path: 'HostConfig.RestartPolicy.Name',
          after: '"always"',
        }),
      );
      await f.api(`/admin/stack/${accepted.id}/action`, 'POST', {
        action: 'accept_configuration',
        revision: review.revision,
      });
      expect(
        (await f.stack()).items.find((row) => row.id === accepted.id).drift,
      ).toBe(false);
      for (const action of ['stop', 'start'])
        expect(
          controller(`/${accepted.id}/action`, 'POST', { action }).status,
        ).toBe(200);
      await release(accepted.id);
    },
  );
  await scenario(
    'auto-remove adoption blocks stop and restart, stays out of backups and permits release',
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
      const created = controller('/backups', 'POST', {
        passphrase: 'service safety fixture passphrase',
      });
      expect(created.status).toBe(200);
      const skipped = JSON.parse(created.body);
      expect(skipped.excluded.map((row) => row.key)).toEqual([accepted.id]);
      await expect
        .poll(() => backupStage(skipped.id), { timeout: 180000 })
        .toBe('complete');
      await healthy();
      expect(
        JSON.parse(docker('inspect', attached.container))[0].State.Running,
      ).toBe(true);
      // Reproduce a journal left by an interrupted imported lifecycle, then let Docker --rm remove it.
      const journal = updateJournal(accepted.id, (journal) => {
        journal.phase = 'changing';
      });
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
        await f.upstream(
          'seerr',
          `user?take=100&q=${encodeURIComponent(email)}`,
        )
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
        await f.upstream(
          'seerr',
          `user?take=100&q=${encodeURIComponent(email)}`,
        )
      ).results.find((row) => row.email === email);
      expect(resolved.id).not.toBe(original.id);
      await f.upstream('seerr', `user/${resolved.id}`, 'DELETE');
      await f.api('/seerr/requests');
      const recovered = (
        await f.upstream(
          'seerr',
          `user?take=100&q=${encodeURIComponent(email)}`,
        )
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
        (
          await f.upstream(
            'seerr',
            `user?take=100&q=${encodeURIComponent(email)}`,
          )
        ).results.find((row) => row.email === email).id,
      ).toBe(recovered.id);
    },
  );
  await scenario(
    'Seerr managed state in Docker storage supports backups',
    async () => {
      const created = controller('/backups', 'POST', {
        passphrase: 'managed service safety fixture passphrase',
      });
      expect(created.status).toBe(200);
      const id = JSON.parse(created.body).id;
      await expect
        .poll(
          () => {
            try {
              return JSON.parse(controller('/backups').body).items.find(
                (row) => row.id === id,
              )?.stage;
            } catch {
              return 'restarting';
            }
          },
          { timeout: 180000 },
        )
        .toBe('complete');
    },
  );
  result.passed = true;
} finally {
  writeFileSync(
    `${output}/${seerrOnly ? 'seerr-result' : 'result'}.json`,
    JSON.stringify(result, null, 2),
  );
  await f.close();
}
