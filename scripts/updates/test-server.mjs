import { expect } from '@playwright/test';
import { join, posix } from 'node:path';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { docker, save, envelope } from './build.mjs';
import { until, refreshDownloads } from './lab.mjs';
import {
  budgets,
  serverToolsReady,
  waitForServerTools,
  waitForState,
} from '../ci-readiness.mjs';

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
  const toolSelections = async () =>
    (await api('/admin/tools')).items.map(
      ({ id, installed, integrity_error }) => ({
        id,
        installed,
        integrity_error,
      }),
    );
  const startupTimeout = budgets.startup;
  await waitForServerTools(() => api('/admin/tools'));
  const originalTools = await toolSelections();
  expect(originalTools).toHaveLength(4);
  for (const tool of originalTools) {
    expect(tool.installed).toBeTruthy();
    expect(tool.integrity_error).toBeNull();
  }
  save(join(output, 'server-tools-before.json'), originalTools);
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
  const transitionTimeout = budgets.provision;
  const transitions = [];
  const observed = new Map();
  const operations = () => {
    const value = controller('/stack/product');
    for (const operation of value.items) {
      if (observed.get(operation.id) === operation.stage) continue;
      observed.set(operation.id, operation.stage);
      transitions.push({ at: new Date().toISOString(), ...operation });
      save(join(output, 'server-transitions.json'), transitions);
      console.log(`Server update ${operation.id}: ${operation.stage}`);
    }
    return value;
  };
  const marker = (file, action) =>
    docker(
      'exec',
      component('server'),
      'python3',
      '-c',
      `from pathlib import Path; p=Path('/var/lib/thelxinoe/${file}'); ${action}`,
    );
  const stage = async (id, wanted) =>
    (
      await waitForState(
        `Server update ${id} reaches ${wanted}`,
        async () => {
          // The server is deliberately stopped during snapshots and replacement.
          const operation = operations().items.find((item) => item.id === id);
          if (operation?.stage === wanted) {
            // The journal can settle before the restarted server reconciles its fence.
            return {
              operation,
              setup: (await request('/setup')).status(),
              tools: await api('/admin/tools'),
            };
          }
          if (
            [
              'blocked',
              'recovery-required',
              'runtime-failure',
              'committed',
              'recovered',
              'restored',
              'superseded',
            ].includes(operation?.stage) &&
            !(wanted === 'restored' && operation?.stage === 'committed')
          )
            throw Object.assign(Error(JSON.stringify(operation)), {
              fatal: true,
            });
          return { operation };
        },
        ({ operation, setup, tools }) =>
          operation?.stage === wanted &&
          setup === 200 &&
          serverToolsReady(tools),
        { timeout: transitionTimeout, interval: 500 },
      )
    ).operation;
  const idleWaits = [];
  const commandWhenIdle = (path, body) =>
    until(async () => {
      try {
        const response = await request(path, 'POST', body);
        const value = await response.json();
        if (response.ok()) return value;
        if (
          response.status() === 409 &&
          [
            'Wait for playback, downloads and background work to finish',
            'Wait for current requests to finish and try again',
          ].includes(value.error?.message)
        ) {
          const jobs = (await api('/admin/jobs')).items.map(
            ({ id, kind, state }) => ({ id, kind, state }),
          );
          idleWaits.push({ at: new Date().toISOString(), path, jobs });
          save(join(output, 'preflight-idle-waits.json'), idleWaits);
          return false;
        }
        throw Error(
          `${path}: HTTP ${response.status()} ${JSON.stringify(value)}`,
        );
      } catch (error) {
        // Response loss is ambiguous: never replay an accepted mutation.
        throw Object.assign(error, { fatal: true });
      }
    }, budgets.restart);
  const ready = async () => {
    const update = await commandWhenIdle('/admin/product-update/prepare');
    await stage(update.id, 'ready');
    return update;
  };
  const activate = (update) =>
    commandWhenIdle(`/admin/product-update/${update.id}/activate`, {
      confirm: true,
    });
  const recreate = async (version) => {
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
    const before = controller('/stack/product').generation;
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
    await until(
      async () => (await api('/health')).version === version,
      startupTimeout,
    );
    await until(() => controller('/health').version === version);
    const after = controller('/stack/product').generation;
    expect(after).toBeGreaterThan(before);
    return after;
  };
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
    'Administrators receive update dots in open clients',
    async () => {
      await mode('candidate');
      await api('/admin/product-update/check', 'POST');
      expect(
        (await api('/me/attention')).items.some(
          (item) => item.target === 'server',
        ),
      ).toBe(true);
      await expect(
        page
          .getByRole('link', { name: 'Settings', exact: true })
          .locator('[data-attention-severity]'),
      ).toBeVisible();
      await page.getByRole('link', { name: 'Settings', exact: true }).click();
      await page
        .getByRole('link', { name: 'Server', exact: true })
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
          .getByRole('link', { name: 'Settings', exact: true })
          .click();
        await native.page
          .getByRole('link', { name: 'Server', exact: true })
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
            await api('/me/attention', 'GET', undefined, ordinary.request)
          ).items.some((n) => n.target === 'server'),
        ).toBe(false);
      } finally {
        await ordinary.close();
      }
    },
  );
  await api('/admin/settings', 'PUT', { timezone: 'Europe/Paris' });
  await scenario(
    'Automatic updates requalify a ready preflight after Compose recreation',
    async () => {
      const previous = await ready();
      const generation = await recreate(lab.base);
      // Retain the activation generation check independently of scheduling.
      expect(
        (
          await request(
            `/admin/product-update/${previous.id}/activate`,
            'POST',
            { confirm: true },
          )
        ).status(),
      ).toBe(409);
      await until(async () => (await request('/setup')).ok());
      await api('/admin/product-update/policy', 'POST', {
        policy: 'automatic',
        window_start: 0,
        window_end: 0,
      });
      const replacement = await until(
        () =>
          operations().items.find(
            (item) =>
              item.id !== previous.id &&
              item.version === lab.next &&
              item.source_generation === generation,
          ),
        startupTimeout,
      );
      await stage(replacement.id, 'committed');
      expect((await api('/health')).version).toBe(lab.next);
      expect(controller('/health').version).toBe(lab.next);
      expect(
        (await state()).controller.items.find((item) => item.id === previous.id)
          .stage,
      ).toBe('superseded');
      await page.screenshot({
        path: join(output, 'server-automatic-after-recreation.png'),
      });
      await api('/admin/product-update/policy', 'POST', {
        policy: 'notify',
        window_start: 0,
        window_end: 0,
      });
      await commandWhenIdle(`/admin/product-update/${replacement.id}/recover`, {
        confirm: true,
      });
      await stage(replacement.id, 'restored');
      expect((await api('/health')).version).toBe(lab.base);
      await api('/admin/product-update/policy', 'POST', {
        policy: 'notify',
        window_start: 0,
        window_end: 0,
      });
    },
  );
  await scenario(
    'One web click waits for playback, survives restart, and recovers failed validation',
    async () => {
      // The earlier upgrade may retain blobs after image references are removed.
      // Publish fresh signed, registry-only layers so both pulls transfer bytes.
      await refreshDownloads(lab);
      envelope(lab, lab.next, lab.images[lab.next]);
      save(join(lab.root, 'lab.json'), lab);
      writeFileSync(join(lab.root, 'registry-control/hold'), 'hold');
      await mode('slow-download');
      await api('/admin/product-update/check', 'POST');
      for (const part of ['server', 'controller']) {
        expect(() =>
          docker('image', 'inspect', lab.images[lab.next][part].reference),
        ).toThrow();
      }
      // Real direct-play session: the queue must wait without holding up playback.
      const ffmpeg = (await toolSelections()).find(
        (tool) => tool.id === 'ffmpeg',
      );
      const executable = docker(
        'exec',
        component('server'),
        'python3',
        '-c',
        `import json; from pathlib import Path; root=Path('/var/lib/thelxinoe/tools/packages/${ffmpeg.installed.id}'); print(root / json.loads((root / 'manifest.json').read_text())['executables']['ffmpeg'])`,
      );
      docker(
        'exec',
        component('server'),
        executable,
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
          name: 'Retry server update',
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
      const ticks = () =>
        Number(marker('lab-product-ticks', 'print(p.read_text())'));
      const before = ticks();
      await until(async () => {
        if (
          (await state()).controller.items.some((item) => item.id === update.id)
        )
          throw Object.assign(Error('Update started during playback'), {
            fatal: true,
          });
        return ticks() >= before + 2;
      });
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'Account', exact: true })
        .click();
      await api(`/playback/${playback.id}/progress`, 'POST', {
        sequence: 0,
        position: 0,
        state: 'stopped',
      });
      docker('restart', component('server'));
      await until(
        async () => (await state()).request?.id === update.id,
        startupTimeout,
      );
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'Server', exact: true })
        .click();
      const downloading = await until(async () => {
        const operation = operations().items.find(
          (item) => item.id === update.id,
        );
        return (
          operation?.stage === 'preparing' &&
          operation.download?.received > 0 &&
          (!operation.download.total ||
            operation.download.received < operation.download.total) &&
          operation
        );
      }, 45000);
      const version = page.getByLabel('Product version', { exact: true });
      await expect(
        version.getByRole('button', {
          name: `Downloading ${downloading.download.component} update`,
          exact: true,
        }),
      ).toBeVisible();
      const progress = version.getByRole('progressbar');
      if (downloading.download.total) {
        await expect
          .poll(async () =>
            Number(await progress.getAttribute('aria-valuenow')),
          )
          .toBeGreaterThan(0);
        expect(
          Number(await progress.getAttribute('aria-valuenow')),
        ).toBeLessThan(100);
      } else {
        await expect(progress).toHaveCount(0);
      }
      await page.screenshot({ path: join(output, 'server-downloading.png') });
      // Restart after controller acceptance, while slow image pulls are ongoing.
      docker('restart', component('server'));
      await until(async () => (await request('/health')).ok(), startupTimeout);
      expect((await request('/playback', 'POST', {})).status()).toBe(503);
      expect(
        (await request('/catalog/roots/missing/scan', 'POST', {})).status(),
      ).toBe(503);
      // Keep the real maintenance fence active while a fresh page boots.
      // A transient 503 must not strand an authenticated user at sign-in.
      expect((await request('/setup')).status()).toBe(503);
      const maintenanceBoot = page.waitForResponse(
        (response) =>
          response.url().endsWith('/api/v1/setup') && response.status() === 503,
      );
      await page.reload();
      await maintenanceBoot;
      await expect(page.getByText('Connecting to your library…')).toBeVisible();
      await expect(
        page.getByRole('button', { name: 'Sign in', exact: true }),
      ).toHaveCount(0);
      await page.screenshot({
        path: join(output, 'server-maintenance-reload.png'),
      });
      rmSync(join(lab.root, 'registry-control/hold'), { force: true });
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
        .getByRole('link', { name: 'Server', exact: true })
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
              '--label',
              `app.thelxinoe.update-lab=${lab.id}`,
              '--network',
              'none',
              '--read-only',
              '-v',
              `${posix.join(lab.storage.replaceAll('\\', '/'), 'server')}:/state:ro`,
              '--entrypoint',
              'python3',
              lab.images[lab.base].server.reference,
              '-c',
              "from pathlib import Path; print(Path('/state/held-validation-entered').exists())",
            ) === 'True',
          transitionTimeout,
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
  const obsoletePreflight = await ready();
  await scenario(
    'UI installs both server and controller and preserves sessions and preferences',
    async () => {
      let webReloads = 0;
      const navigated = (request) => {
        if (
          request.isNavigationRequest() &&
          request.frame() === page.mainFrame()
        )
          webReloads++;
      };
      page.on('request', navigated);
      const previous = (await state()).request?.id;
      const client = native?.page ?? page;
      await client
        .getByRole('button', {
          name: `Update server to ${lab.next}`,
          exact: true,
        })
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
      const tools = await toolSelections();
      expect(tools).toEqual(originalTools);
      save(join(output, 'server-tools-after.json'), tools);
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
      page.off('request', navigated);
      await expect(
        page.getByLabel('Product version', { exact: true }).locator('strong'),
      ).toHaveText(lab.next, { timeout: 45000 });
      await page.screenshot({ path: join(output, 'server-committed.png') });
      await expect(
        page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole('button', { name: 'Reload web app', exact: true }),
      ).toHaveCount(0);
      lab.committed = update.id;
    },
  );
  await scenario(
    'Obsolete recovery is rejected without poisoning the journal or controller startup',
    async () => {
      expect(
        (
          await request(
            `/admin/product-update/${obsoletePreflight.id}/recover`,
            'POST',
            { confirm: true },
          )
        ).status(),
      ).toBe(409);
      expect(
        (await state()).controller.items.find(
          (item) => item.id === obsoletePreflight.id,
        ).stage,
      ).toBe('ready');
      docker('restart', component('controller'));
      await until(() => controller('/health').version === lab.next);
      expect(
        (await state()).controller.items.find(
          (item) => item.id === obsoletePreflight.id,
        ).stage,
      ).toBe('ready');
      expect((await api('/health')).version).toBe(lab.next);
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
      await until(
        async () => (await api('/health')).version === lab.next,
        startupTimeout,
      );
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
      await expect(
        page.locator('header').getByText('Connected', { exact: true }),
      ).toBeVisible({
        timeout: 45000,
      });
    },
  );
  await scenario(
    'Explicit restore returns both binaries and all server state to the snapshot',
    async () => {
      await page.getByRole('link', { name: 'Settings', exact: true }).click();
      await page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'Server', exact: true })
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
      marker('hold-release-request', "p.write_text('hold')");
      const heldRequest = request('/admin/settings');
      try {
        await until(
          () =>
            marker('held-release-request-entered', 'print(p.exists())') ===
            'True',
        );
        const rejected = page.waitForResponse(
          (response) =>
            response.url().endsWith(`/${lab.committed}/recover`) &&
            response.status() === 409,
        );
        const button = restore.getByRole('button', {
          name: `Restore ${lab.base}`,
          exact: true,
        });
        await button.click();
        expect((await rejected).status()).toBe(409);
        await expect(button).toBeDisabled();
        await page.screenshot({ path: join(output, 'restore-waiting.png') });
      } finally {
        marker('hold-release-request', 'p.unlink(missing_ok=True)');
        await heldRequest;
      }
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
      // Old obsolete attempts must not block another update after recovery.
      await ready();
    },
  );
}
