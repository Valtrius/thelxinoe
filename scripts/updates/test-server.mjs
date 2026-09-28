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
        .getByRole('button', { name: 'Open server updates', exact: true })
        .first()
        .click();
      await expect(
        page.getByRole('region', { name: 'Product updates' }),
      ).toContainText(`Version ${lab.next}`);
      if (native) {
        await native.page
          .getByRole('button', { name: 'Notifications', exact: true })
          .click();
        await native.page
          .getByRole('button', { name: 'Open server updates', exact: true })
          .first()
          .click();
        await expect(
          native.page.getByRole('region', { name: 'Product updates' }),
        ).toContainText(`Version ${lab.next}`);
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
              '/admin/product-update/prepare',
              'POST',
              {},
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
    'Failed live validation restores state and open clients reconnect',
    async () => {
      const update = await ready();
      const epoch = (await api('/auth/event-ticket', 'POST')).epoch;
      marker(
        'fail-live-validation',
        "p.write_text('fail after state mutation')",
      );
      await activate(update);
      await stage(update.id, 'recovered');
      expect((await api('/health')).version).toBe(lab.base);
      expect((await api('/admin/diagnostics')).database_ok).toBe(true);
      expect((await api('/admin/settings')).timezone).toBe('Europe/Paris');
      expect(marker('failed-validation-mutated', 'print(p.exists())')).toBe(
        'False',
      );
      expect((await api('/auth/event-ticket', 'POST')).epoch).not.toBe(epoch);
      await expect(
        page.getByRole('region', { name: 'Product updates' }),
      ).toContainText('recovered', { timeout: 45000 });
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
      const region = page.getByRole('region', { name: 'Product updates' });
      await region
        .getByRole('button', { name: 'Prepare and test release', exact: true })
        .click();
      await expect(
        region.getByRole('button', {
          name: 'Install prepared release',
          exact: true,
        }),
      ).toBeVisible({ timeout: 300000 });
      const update = (await state()).controller.items.find(
        (u) => u.stage === 'ready',
      );
      const client = native
        ? native.page.getByRole('region', { name: 'Product updates' })
        : region;
      await expect(
        client.getByRole('button', {
          name: 'Install prepared release',
          exact: true,
        }),
      ).toBeVisible({ timeout: 30000 });
      await client
        .getByRole('button', { name: 'Install prepared release', exact: true })
        .click();
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
      await expect(
        page.getByRole('button', { name: 'Reload web app', exact: true }),
      ).toBeVisible({ timeout: 45000 });
      await page.screenshot({ path: join(output, 'server-committed.png') });
      await page
        .getByRole('button', { name: 'Reload web app', exact: true })
        .click();
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
      await api(`/admin/product-update/${lab.committed}/recover`, 'POST', {
        confirm: true,
      });
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
