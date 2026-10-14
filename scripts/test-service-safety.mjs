import { expect } from '@playwright/test';
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
/** Review a Connected service's transfer without approving it, then close the review. */
async function reviewOnly(attached) {
  const { review } = await f.transfer(attached.id, { approve: false });
  await f.api(`/admin/transfers/${review.id}/cancel`, 'POST');
  return review;
}
async function forget(attached) {
  await f.api(`/admin/services/${attached.id}/forget`, 'POST');
  docker('rm', '-f', attached.container);
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
try {
  for (const layout of [
    'config-contains-media',
    'media-contains-config',
    'server-secondary-media',
  ]) {
    await scenario(
      `transfer review rejects ${layout} and leaves the original running`,
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
        const before = startedAt(attached.container);
        const review = await reviewOnly(attached);
        expect(review.review.blockers.join(' ')).toMatch(/overlaps/);
        // Nothing stops or copies while the layout is unsupported.
        expect(startedAt(attached.container)).toBe(before);
        expect(
          JSON.parse(docker('inspect', attached.container))[0].State.Running,
        ).toBe(true);
        expect((await f.stack()).provisions).toEqual([]);
        await forget(attached);
      },
    );
  }
  await scenario(
    'an auto-remove original is rejected at review and keeps running',
    async () => {
      const attached = await f.attach(
        'radarr',
        '/existing-radarr',
        imageManifest.services.radarr,
        { existingLayout: true, autoRemove: true },
      );
      const before = startedAt(attached.container);
      const review = await reviewOnly(attached);
      expect(review.review.blockers.join(' ')).toContain('auto-remove');
      expect(startedAt(attached.container)).toBe(before);
      await forget(attached);
    },
  );
  await scenario(
    'transferred state is backed up and restored only when chosen while the original stays untouched',
    async () => {
      const attached = await f.attach(
        'radarr',
        '/existing-radarr',
        imageManifest.services.radarr,
        { existingLayout: true },
      );
      const { view } = await f.transfer(attached.id);
      expect(view.state, view.error ?? '').toBe('complete');
      const provision = (await f.api('/admin/services')).items.find(
        (item) => item.kind === 'radarr',
      ).ownership.id;
      const replacement = async () =>
        (await f.stack()).items.find((item) => item.kind === 'radarr')
          .container_id;
      const marker = async () =>
        docker('exec', await replacement(), 'cat', '/config/safety-marker');
      const original = f.appdataDigest(attached.appdata);
      docker(
        'exec',
        '-u',
        '0',
        await replacement(),
        'sh',
        '-c',
        'printf before > /config/safety-marker',
      );
      const passphrase = 'service safety fixture passphrase';
      const created = controller('/backups', 'POST', { passphrase });
      expect(created.status).toBe(200);
      const id = JSON.parse(created.body).id;
      expect(
        JSON.parse(created.body).components.map((row) => row.key),
      ).toContain(provision);
      const stage = () => backupStage(id);
      await expect.poll(stage, { timeout: 180000 }).toBe('complete');
      docker(
        'exec',
        '-u',
        '0',
        await replacement(),
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
        controller(`/backups/${id}/restore`, 'POST', {
          passphrase,
          components: [provision],
        }).status,
      ).toBe(200);
      await expect.poll(stage, { timeout: 180000 }).toBe('restored');
      expect(await marker()).toBe('before');
      expect(
        f.compose('exec', '-T', 'server', 'cat', '/media/movies/safety-media'),
      ).toBe('latest');
      // The staged swap leaves no workspace behind in the service configuration.
      expect(
        docker('exec', await replacement(), 'ls', '-A', '/config'),
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
      // Backups and restores never touch the retained original's appdata.
      expect(f.appdataDigest(attached.appdata)).toBe(original);
      await healthy();
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
