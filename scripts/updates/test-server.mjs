import { expect } from '@playwright/test';
import { join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { docker, save } from './build.mjs';
import { until } from './lab.mjs';

export async function serverScenarios({
  lab,
  page,
  context,
  api,
  request,
  native,
  scenario,
  mode,
  output,
}) {
  const state = () => api('/admin/product-update');
  const component = (name) => `${lab.id}-${name}`;
  const controller = (path) =>
    JSON.parse(
      docker(
        'exec',
        component('controller'),
        'curl',
        '-fsS',
        '--unix-socket',
        '/run/thelxinoe/controller.sock',
        'http://localhost' + path,
      ),
    );
  const marker = (file, action) =>
    docker(
      'exec',
      component('server'),
      'python3',
      '-c',
      `from pathlib import Path; p=Path('/var/lib/thelxinoe/${file}'); ${action}`,
    );
  const stage = async (id, wanted) =>
    until(async () => {
      const operation = (await state()).controller.items.find(
        (item) => item.id === id,
      );
      if (operation?.stage === wanted) return operation;
      if (
        ['blocked', 'recovery-required', 'runtime-failure'].includes(
          operation?.stage,
        )
      )
        throw Object.assign(Error(JSON.stringify(operation)), { fatal: true });
      return false;
    }, 300000);
  const ready = async () => {
    const update = await api('/admin/product-update/prepare', 'POST');
    await stage(update.id, 'ready');
    return update;
  };
  const activate = (update) =>
    api(`/admin/product-update/${update.id}/activate`, 'POST', {
      confirm: true,
    });
  await api('/admin/product-update/policy', 'POST', {
    policy: 'notify',
    window_start: 0,
    window_end: 0,
  });
  await scenario(
    'Current release metadata is cached and HTTPS redirects work',
    async () => {
      await mode('base');
      await api('/admin/product-update/check', 'POST');
      expect((await state()).release).toBeNull();
      const envelope = (await api('/release')).envelope;
      expect(JSON.parse(Buffer.from(envelope.payload, 'base64')).version).toBe(
        lab.base,
      );
    },
  );
  await scenario(
    'Bad or unavailable feeds clear candidates and schedule a retry',
    async () => {
      for (const fault of ['tampered', 'expired', 'unavailable']) {
        await mode('candidate');
        await api('/admin/product-update/check', 'POST');
        expect((await state()).release.version).toBe(lab.next);
        await mode(fault);
        expect(
          (await request('/admin/product-update/check', 'POST')).ok(),
        ).toBe(false);
        const value = await state();
        expect(value.release).toBeNull();
        expect(value.observation.error).toBeTruthy();
        expect(
          value.observation.next_check - value.observation.checked_at,
        ).toBeLessThanOrEqual(1800);
      }
    },
  );
  await scenario(
    'Administrators receive actionable notices in open clients',
    async () => {
      await mode('candidate');
      await api('/admin/product-update/check', 'POST');
      expect(
        (await api('/me/notifications')).items.some(
          (item) => item.target === 'server',
        ),
      ).toBe(true);
      await page
        .getByRole('button', { name: 'Notifications', exact: true })
        .click();
      await page
        .getByRole('button', { name: 'View server version', exact: true })
        .first()
        .click();
      await expect(
        page.getByRole('button', {
          name: `Update server to ${lab.next}`,
          exact: true,
        }),
      ).toBeVisible();
      if (native) {
        await native.page
          .getByRole('button', { name: 'Notifications', exact: true })
          .click();
        await native.page
          .getByRole('button', { name: 'View server version', exact: true })
          .first()
          .click();
        await expect(
          native.page.getByRole('button', {
            name: `Update server to ${lab.next}`,
            exact: true,
          }),
        ).toBeVisible();
      }
      await page.screenshot({ path: join(output, 'server-notice.png') });
    },
  );
  await scenario(
    'Ordinary users cannot install server releases or see admin notices',
    async () => {
      await api('/users', 'POST', {
        username: 'viewer',
        password: 'viewer update lab passphrase',
        role: 'user',
      });
      const ordinary = await context.browser().newContext();
      try {
        await api(
          '/auth/login',
          'POST',
          { username: 'viewer', password: 'viewer update lab passphrase' },
          ordinary.request,
        );
        expect(
          (
            await request(
              '/admin/product-update/install',
              'POST',
              { version: lab.next },
              ordinary.request,
            )
          ).status(),
        ).toBe(403);
        expect(
          (
            await api('/me/notifications', 'GET', undefined, ordinary.request)
          ).items.some((n) => n.target === 'server'),
        ).toBe(false);
      } finally {
        await ordinary.close();
      }
    },
  );
  await api('/admin/settings', 'PUT', { timezone: 'Europe/Paris' });
  await scenario(
    'One web click waits for playback, survives restart, and recovers failed validation',
    async () => {
      await mode('slow-download');
      // Remove only this lab's candidate references so preparation must stream
      // real pulls from its registry instead of taking the local-image shortcut.
      for (const part of ['server', 'controller']) {
        try {
          docker(
            'image',
            'rm',
            `localhost:${lab.registryPort}/${lab.id}/${part}:${lab.next}`,
          );
        } catch {
          /* A retained lab may have only the digest reference. */
        }
        try {
          docker('image', 'rm', lab.images[lab.next][part].reference);
        } catch {
          /* Removing the only tag may already have removed the digest. */
        }
        expect(() =>
          docker('image', 'inspect', lab.images[lab.next][part].reference),
        ).toThrow();
      }
      // Real direct-play session: the queue must wait without holding up playback.
      docker(
        'exec',
        component('server'),
        'ffmpeg',
        '-v',
        'error',
        '-f',
        'lavfi',
        '-i',
        'color=c=black:s=160x90:r=1',
        '-t',
        '4',
        '-c:v',
        'libx264',
        '-pix_fmt',
        'yuv420p',
        '-y',
        '/media/Update Lab.mp4',
      );
      const root = await api('/catalog/roots', 'POST', {
        name: 'Update lab',
        kind: 'movies',
        path: '/media',
      });
      const scan = await api(`/catalog/roots/${root.id}/scan`, 'POST');
      await until(
        async () =>
          (await api('/admin/jobs')).items.find((job) => job.id === scan.job_id)
            ?.state === 'complete',
      );
      const media = (await api('/catalog?kind=movie')).items[0];
      const playback = await api('/playback', 'POST', {
        media_id: media.id,
        position: 0,
        options: {
          quality: 'auto',
          audio: null,
          subtitle: null,
          capabilities: {
            containers: ['mp4'],
            video: ['h264'],
            audio: ['aac'],
            hls: true,
          },
        },
      });
      expect(
        (
          await request('/admin/product-update/install', 'POST', {
            version: '99.0.0',
          })
        ).status(),
      ).toBe(409);
      const epoch = (await api('/auth/event-ticket', 'POST')).epoch;
      marker(
        'fail-live-validation',
        "p.write_text('fail after state mutation')",
      );
      await page
        .getByRole('button', {
          name: `Update server to ${lab.next}`,
          exact: true,
        })
        .click();
      await expect(
        page.getByRole('button', {
          name: 'Waiting for the server to be idle',
          exact: true,
        }),
      ).toBeVisible();
      const update = (await state()).request;
      const duplicate = await api('/admin/product-update/install', 'POST', {
        version: lab.next,
      });
      expect(duplicate.id).toBe(update.id);
      // More than one coordinator tick must pass with no controller operation.
      const untilTime = Date.now() + 11000;
      await until(async () => {
        if (
          (await state()).controller.items.some((item) => item.id === update.id)
        )
          throw Object.assign(Error('Update started during playback'), {
            fatal: true,
          });
        return Date.now() >= untilTime;
      });
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Account', exact: true })
        .click();
      await api(`/playback/${playback.id}/progress`, 'POST', {
        sequence: 0,
        position: 0,
        state: 'stopped',
      });
      docker('restart', component('server'));
      await until(async () => (await state()).request?.id === update.id);
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Server', exact: true })
        .click();
      const progress = page
        .getByLabel('Product version', { exact: true })
        .getByRole('progressbar');
      await expect
        .poll(
          async () =>
            Number(
              await progress.getAttribute('aria-valuenow', { timeout: 15000 }),
            ),
          { timeout: 45000 },
        )
        .toBeGreaterThan(0);
      expect(Number(await progress.getAttribute('aria-valuenow'))).toBeLessThan(
        100,
      );
      await page.screenshot({ path: join(output, 'server-downloading.png') });
      const recovered = await stage(update.id, 'recovered');
      expect(recovered.download.component).toBe('controller');
      expect(recovered.download.received).toBeGreaterThan(0);
      expect(recovered.download.received).toBe(recovered.download.total);
      await mode('candidate');
      expect((await api('/health')).version).toBe(lab.base);
      expect((await api('/admin/diagnostics')).database_ok).toBe(true);
      expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
      expect(marker('failed-validation-mutated', 'print(p.exists())')).toBe(
        'False',
      );
      expect((await api('/auth/event-ticket', 'POST')).epoch).not.toBe(epoch);
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Server', exact: true })
        .click();
      await expect(
        page.getByRole('button', { name: 'Retry server update', exact: true }),
      ).toBeVisible({ timeout: 45000 });
      await expect(
        page.getByLabel('Product version', { exact: true }).locator('strong'),
      ).toHaveText(lab.base);
      marker('fail-live-validation', 'p.unlink(missing_ok=True)');
    },
  );
  await scenario(
    'Controller interruption recovers with the image registry offline',
    async () => {
      const update = await ready();
      marker('hold-live-validation', "p.write_text('hold')");
      docker('stop', component('registry'));
      try {
        await activate(update);
        // The old server is stopped, so inspect the state volume with a cached image.
        await until(
          () =>
            docker(
              'run',
              '--rm',
              '--network',
              'none',
              '--read-only',
              '-v',
              `${join(lab.storage, 'server')}:/state:ro`,
              '--entrypoint',
              'python3',
              lab.images[lab.base].server.reference,
              '-c',
              "from pathlib import Path; print(Path('/state/held-validation-entered').exists())",
            ) === 'True',
          90000,
        );
        docker('restart', '--time', '0', component('controller'));
        await stage(update.id, 'recovered');
        expect((await api('/health')).version).toBe(lab.base);
        expect(marker('held-validation-entered', 'print(p.exists())')).toBe(
          'False',
        );
        expect(
          JSON.parse(docker('inspect', component('registry')))[0].State.Running,
        ).toBe(false);
      } finally {
        docker('start', component('registry'));
      }
      marker('hold-live-validation', 'p.unlink(missing_ok=True)');
    },
  );
  await scenario(
    'UI installs both server and controller and preserves sessions and preferences',
    async () => {
      let webReloads = 0;
      const navigated = (frame) => {
        if (frame === page.mainFrame()) webReloads++;
      };
      page.on('framenavigated', navigated);
      const previous = (await state()).request?.id;
      const client = native?.page ?? page;
      await client
        .getByRole('button', { name: 'Retry server update', exact: true })
        .click();
      const update = await until(async () => {
        const intent = (await state()).request;
        return intent?.id !== previous && intent;
      });
      await stage(update.id, 'committed');
      expect((await api('/health')).version).toBe(lab.next);
      expect(controller('/health').version).toBe(lab.next);
      for (const part of ['server', 'controller'])
        expect(
          JSON.parse(docker('inspect', component(part)))[0].Config.Labels[
            'org.opencontainers.image.version'
          ],
        ).toBe(lab.next);
      expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
      if (native)
        expect(
          (
            await native.invoke('backend_request', {
              path: '/auth/me',
              method: 'GET',
              body: null,
            })
          ).status,
        ).toBe(200);
      if (native) {
        await expect(
          native.page
            .getByLabel('Product version', { exact: true })
            .locator('strong'),
        ).toHaveText(lab.next, { timeout: 45000 });
      }
      await expect
        .poll(() => webReloads, { timeout: 45000 })
        .toBeGreaterThan(0);
      page.off('framenavigated', navigated);
      await expect(
        page.getByLabel('Product version', { exact: true }).locator('strong'),
      ).toHaveText(lab.next, { timeout: 45000 });
      await page.screenshot({ path: join(output, 'server-committed.png') });
      await expect(
        page.getByRole('button', { name: 'Settings', exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole('button', { name: 'Reload web app', exact: true }),
      ).toHaveCount(0);
      lab.committed = update.id;
    },
  );
  await scenario(
    'Accepted Compose pins survive actual container recreation',
    async () => {
      const directory = join(lab.root, 'accepted');
      mkdirSync(directory, { recursive: true });
      for (const file of [
        'compose.yaml',
        'compose.override.yaml',
        'desired-state.json',
      ])
        writeFileSync(
          join(directory, file),
          docker(
            'exec',
            component('controller'),
            'cat',
            '/var/lib/thelxinoe/deployment/' + file,
          ),
        );
      const before = JSON.parse(
        docker(
          'exec',
          component('controller'),
          'cat',
          '/var/lib/thelxinoe/deployment/desired-state.json',
        ),
      );
      save(join(directory, 'compose.yaml'), {
        name: lab.id,
        services: {
          server: { image: 'thelxinoe-server:stale-bootstrap' },
          controller: { image: 'thelxinoe-controller:stale-bootstrap' },
        },
      });
      docker(
        'compose',
        '--project-directory',
        directory,
        'up',
        '-d',
        '--force-recreate',
        '--no-build',
        '--pull',
        'never',
      );
      await until(async () => (await api('/health')).version === lab.next);
      await until(() => controller('/health').version === lab.next);
      const after = JSON.parse(
        docker(
          'exec',
          component('controller'),
          'cat',
          '/var/lib/thelxinoe/deployment/desired-state.json',
        ),
      );
      expect(after.generation).toBeGreaterThan(before.generation);
      expect(after.server.Image).toBe(before.server.Image);
      expect(after.controller.Image).toBe(before.controller.Image);
      expect(after.server.Id).not.toBe(before.server.Id);
      await expect(page.getByText('Connected', { exact: true })).toBeVisible({
        timeout: 45000,
      });
    },
  );
  await scenario(
    'Explicit restore returns both binaries and all server state to the snapshot',
    async () => {
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Server', exact: true })
        .click();
      // Force the connected client's event cursor beyond the restored database.
      for (let i = 0; i < 30; i++)
        await api('/admin/settings', 'PUT', { timezone: 'Asia/Tokyo' });
      await expect(
        page.getByRole('combobox', {
          name: 'Server default timezone',
          exact: true,
        }),
      ).toHaveValue('Asia/Tokyo', { timeout: 45000 });
      await page
        .getByLabel('Product version', { exact: true })
        .getByRole('button', { name: 'Rollback', exact: true })
        .click();
      const restore = page.getByRole('dialog');
      await expect(restore).toContainText(`Rollback to ${lab.base}`);
      await restore.getByLabel('Type RESTORE to continue').fill('RESTORE');
      await restore
        .getByRole('button', { name: `Restore ${lab.base}`, exact: true })
        .click();
      await stage(lab.committed, 'restored');
      expect((await api('/health')).version).toBe(lab.base);
      expect(controller('/health').version).toBe(lab.base);
      expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
      await expect(
        page.getByRole('combobox', {
          name: 'Server default timezone',
          exact: true,
        }),
      ).toHaveValue('Europe/Paris', { timeout: 45000 });
      await api('/admin/settings', 'PUT', { timezone: 'America/New_York' });
      await expect(
        page.getByRole('combobox', {
          name: 'Server default timezone',
          exact: true,
        }),
      ).toHaveValue('America/New_York', { timeout: 45000 });
      await page.screenshot({ path: join(output, 'server-restored.png') });
    },
  );
}
