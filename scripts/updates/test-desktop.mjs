import { chromium, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { join } from 'node:path';
import { existsSync, writeFileSync, rmSync } from 'node:fs';
import { until, installBase } from './lab.mjs';

export async function connectDesktop(lab) {
  return until(async () => {
    const browser = await chromium.connectOverCDP(
      `http://127.0.0.1:${lab.cdpPort}`,
      { timeout: 5000 },
    );
    const context = browser.contexts()[0];
    const page = context
      ?.pages()
      .find((page) => page.url().startsWith('http://tauri.localhost'));
    if (!page) {
      await browser.close();
      return false;
    }
    const invoke = (command, args = {}) =>
      page.evaluate(
        ({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args),
        { command, args },
      );
    try {
      await invoke('desktop_update_status');
    } catch {
      await browser.close();
      return false;
    }
    return { browser, context, page, invoke, close: () => browser.close() };
  });
}

export async function desktopScenarios({
  lab,
  native,
  scenario,
  mode,
  output,
  credentials,
  onConnect,
}) {
  const idle = () =>
    until(
      async () =>
        !['checking', 'downloading', 'waiting', 'installing'].includes(
          (await native.invoke('desktop_update_status')).phase,
        ),
    );
  const check = async () => {
    await idle();
    return native.invoke('desktop_update_check');
  };
  const login = async () => {
    const signOut = native.page.getByRole('button', {
      name: 'Sign out',
      exact: true,
    });
    const username = native.page.getByLabel('Username', { exact: true });
    await expect(signOut.or(username).first()).toBeVisible();
    if (await signOut.isVisible()) await signOut.click();
    await expect(username).toBeVisible();
    await native.page
      .getByLabel('Server address', { exact: true })
      .fill(lab.baseUrl);
    await native.page
      .getByLabel('Username', { exact: true })
      .fill(credentials.username);
    await native.page
      .getByLabel('Password', { exact: true })
      .fill(credentials.password);
    await native.page
      .getByRole('button', { name: 'Sign in', exact: true })
      .click();
    await expect(
      native.page.getByRole('button', { name: 'Settings', exact: true }),
    ).toBeVisible();
    expect(await native.invoke('server_url')).toBe(lab.baseUrl);
  };
  await native.context.tracing.start({ screenshots: true, snapshots: true });
  await scenario(
    'Login hides desktop updates and sign-in connects to the entered server',
    async () => {
      await mode('candidate');
      await native.invoke('change_server', { value: 'http://127.0.0.1:1' });
      await native.page.reload();
      const value = await check();
      expect(value.installed).toBe(lab.base);
      expect(value.release.version).toBe(lab.next);
      await expect(
        native.page.getByRole('button', { name: 'Sign in', exact: true }),
      ).toBeVisible();
      await expect(
        native.page.getByText('Desktop updates', { exact: true }),
      ).toHaveCount(0);
      await expect(
        native.page.getByRole('group', { name: 'Desktop update policy' }),
      ).toHaveCount(0);
      await expect(
        native.page.getByLabel('Desktop version', { exact: true }),
      ).toHaveCount(0);
      await expect(
        native.page.getByLabel('Client update available'),
      ).toHaveCount(0);
      await native.page.screenshot({
        path: join(output, 'desktop-login.png'),
      });
      await login();
      await native.page
        .getByRole('button', { name: 'Settings', exact: true })
        .click();
      await native.page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Desktop', exact: true })
        .click();
      await expect(
        native.page.getByLabel('Desktop version', { exact: true }),
      ).toContainText(lab.base);
      await expect(
        native.page.getByLabel('Server address', { exact: true }),
      ).toHaveValue(lab.baseUrl);
      await expect(
        native.page.getByText('Last check:', { exact: false }),
      ).toHaveCount(0);
      await native.page.screenshot({
        path: join(output, 'desktop-settings.png'),
      });
      await native.page
        .getByRole('button', { name: 'Sign out', exact: true })
        .click();
    },
  );
  await scenario(
    'Desktop rejects expired, forged and unavailable feeds',
    async () => {
      for (const fault of ['expired', 'tampered', 'unavailable']) {
        await mode(fault);
        await expect(check()).rejects.toBeTruthy();
        expect(
          (await native.invoke('desktop_update_status')).release,
        ).toBeNull();
      }
      await mode('candidate');
      await check();
    },
  );
  await scenario(
    'Desktop rejects mismatched metadata and corrupted installer bytes',
    async () => {
      for (const fault of ['mismatch', 'corrupt']) {
        await mode(fault);
        await expect(
          native.invoke('desktop_update_download'),
        ).rejects.toBeTruthy();
        expect((await native.invoke('desktop_update_status')).phase).toBe(
          'idle',
        );
        await expect(native.invoke('desktop_update_install')).rejects.toThrow(
          'Download and verify',
        );
      }
    },
  );
  await scenario(
    'Automatic does not download or install on the login screen',
    async () => {
      await mode('base');
      await check();
      expect((await native.invoke('desktop_update_status')).release).toBeNull();
      await mode('candidate');
      await check();
      await native.invoke('desktop_update_policy', { policy: 'automatic' });
      // Observe a complete native scheduler interval while signed out.
      await native.page.waitForTimeout(65000);
      expect((await native.invoke('desktop_update_status')).phase).toBe('idle');
      expect((await native.invoke('desktop_update_status')).installed).toBe(
        lab.base,
      );
      await native.invoke('desktop_update_policy', { policy: 'notify' });
    },
  );
  await scenario(
    'Incompatible connected server blocks installation while discovery remains available',
    async () => {
      const requests = [];
      const incompatible = createServer((request, response) => {
        requests.push(request.url);
        response.setHeader('Content-Type', 'application/json');
        response.end(
          JSON.stringify(
            request.url === '/api/v1/health'
              ? {
                  status: 'ok',
                  version: '99.0.0',
                  api_version: 99,
                  api_min: 99,
                  api_max: 99,
                }
              : {},
          ),
        );
      });
      await new Promise((done) => incompatible.listen(0, '127.0.0.1', done));
      try {
        await native.page
          .getByLabel('Server address', { exact: true })
          .fill(`http://127.0.0.1:${incompatible.address().port}`);
        await native.page
          .getByLabel('Username', { exact: true })
          .fill(credentials.username);
        await native.page
          .getByLabel('Password', { exact: true })
          .fill(credentials.password);
        await native.page
          .getByRole('button', { name: 'Sign in', exact: true })
          .click();
        await expect(
          native.page.getByRole('heading', { name: 'Update required' }),
        ).toBeVisible();
        expect(requests).toContain('/api/v1/health');
        expect(requests).not.toContain('/api/v1/auth/login');
        await expect(
          native.page.getByLabel('Desktop version', { exact: true }),
        ).toBeVisible();
        await expect(
          native.page.getByRole('group', { name: 'Desktop update policy' }),
        ).toHaveCount(0);
        await mode('candidate');
        await check();
        await native.invoke('desktop_update_download');
        await expect(native.invoke('desktop_update_install')).rejects.toThrow(
          'Update the connected server',
        );
        expect((await native.invoke('desktop_update_status')).phase).toBe(
          'ready',
        );
      } finally {
        await native.invoke('change_server', { value: 'http://127.0.0.1:1' });
        await native.page.reload();
        await new Promise((done) => incompatible.close(done));
      }
    },
  );
  await scenario(
    'Installation drains accepted playback preparation and tool operations',
    async () => {
      await login();
      const rejection = join(lab.root, 'reject-install');
      const drained = rejection + '.entered';
      writeFileSync(rejection, 'reject');
      try {
        for (const [work, command, args] of [
          [
            'playback',
            'mpv_play',
            {
              choice: { id: 'lab-preparing', title: 'Lab preparation' },
              music: false,
            },
          ],
          ['tools', 'tools_install', { packageId: 'lab-held-operation' }],
        ]) {
          const hold = join(lab.root, `hold-${work}`);
          const entered = hold + '.entered';
          rmSync(drained, { force: true });
          writeFileSync(hold, 'hold');
          const working = native.invoke(command, args).then(
            () => 'completed',
            () => 'rejected',
          );
          try {
            await until(() => existsSync(entered));
            let settled = false;
            const installing = native
              .invoke('desktop_update_install')
              .then(
                () => null,
                (error) => String(error),
              )
              .finally(() => {
                settled = true;
              });
            await native.page.waitForTimeout(1000);
            expect(settled, `Installer bypassed accepted ${work}`).toBe(false);
            expect(existsSync(drained)).toBe(false);
            rmSync(hold);
            expect(await working).toBe('rejected');
            expect(await installing).toContain('after draining work');
            expect(existsSync(drained)).toBe(true);
            expect((await native.invoke('desktop_update_status')).phase).toBe(
              'ready',
            );
          } finally {
            rmSync(hold, { force: true });
            rmSync(entered, { force: true });
            await working;
          }
        }
        await native.page.screenshot({
          path: join(output, 'desktop-work-drained.png'),
        });
      } finally {
        rmSync(rejection, { force: true });
        rmSync(drained, { force: true });
      }
    },
  );
  await scenario(
    'Automatic consumes a retained verified installer after compatibility recovers',
    async () => {
      // If apply downloads again, the publisher now returns corrupt bytes.
      await mode('corrupt');
      expect((await check()).phase).toBe('ready');
      await native.context.tracing.stop({
        path: join(output, 'desktop-retained-before-trace.zip'),
      });
      await native.invoke('desktop_update_policy', { policy: 'automatic' });
      const previous = native;
      native = await until(async () => {
        const connected = await connectDesktop(lab);
        if (
          (await connected.invoke('desktop_update_status')).installed !==
          lab.next
        ) {
          await connected.close();
          return false;
        }
        return connected;
      }, 180000);
      onConnect(native);
      await previous.close().catch(() => {});
      expect((await native.invoke('desktop_update_status')).installed).toBe(
        lab.next,
      );
      expect(
        (
          await native.invoke('backend_request', {
            path: '/auth/me',
            method: 'GET',
          })
        ).status,
      ).toBe(200);
      await native.page.screenshot({
        path: join(output, 'desktop-retained-installed.png'),
      });
      await native.invoke('desktop_update_policy', { policy: 'notify' });
      await mode('base');
      await native.close();
      installBase(lab);
      native = await connectDesktop(lab);
      onConnect(native);
      await native.context.tracing.start({
        screenshots: true,
        snapshots: true,
      });
      expect((await native.invoke('desktop_update_status')).installed).toBe(
        lab.base,
      );
      expect(
        (
          await native.invoke('backend_request', {
            path: '/auth/me',
            method: 'GET',
          })
        ).status,
      ).toBe(200);
      await native.page.reload();
    },
  );
  await scenario(
    'One click downloads, installs and relaunches with credentials and settings preserved',
    async () => {
      await login();
      // Clear the prepared installer so this click must download as well as install.
      await mode('base');
      await check();
      await native.page
        .getByRole('button', { name: 'Settings', exact: true })
        .click();
      await native.page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('button', { name: 'Desktop', exact: true })
        .click();
      await mode('slow-download');
      await native.page
        .getByRole('button', { name: 'Check for desktop updates', exact: true })
        .click();
      const update = native.page.getByRole('button', {
        name: `Update desktop to ${lab.next}`,
        exact: true,
      });
      await expect(update).toBeVisible();
      expect((await native.invoke('desktop_update_status')).phase).toBe('idle');
      expect((await native.invoke('desktop_update_status')).installed).toBe(
        lab.base,
      );
      await native.invoke('plugin:window|set_size', {
        label: 'main',
        value: { Physical: { width: 1000, height: 700 } },
      });
      await native.invoke('plugin:window|set_position', {
        label: 'main',
        value: { Physical: { x: 97, y: 61 } },
      });
      const windowState = async () => ({
        position: await native.invoke('plugin:window|outer_position', {
          label: 'main',
        }),
        size: await native.invoke('plugin:window|inner_size', {
          label: 'main',
        }),
        maximized: await native.invoke('plugin:window|is_maximized', {
          label: 'main',
        }),
      });
      const expectedWindow = await windowState();
      expect(expectedWindow.position).toEqual({ x: 97, y: 61 });
      await native.page.screenshot({ path: join(output, 'desktop-ready.png') });
      const hold = join(lab.root, 'hold-desktop-download');
      writeFileSync(hold, 'hold');
      try {
        await update.click();
        await expect(
          native.page.getByRole('progressbar', { name: 'Update download' }),
        ).toBeAttached({ timeout: 10000 });
        await expect
          .poll(async () =>
            Number(
              await native.page
                .getByRole('progressbar', { name: 'Update download' })
                .getAttribute('aria-valuenow'),
            ),
          )
          .toBeGreaterThan(0);
        await native.page.mouse.move(0, 0);
        await expect(
          native.page
            .getByLabel('Desktop version', { exact: true })
            .getByRole('tooltip'),
        ).toHaveCSS('opacity', '0');
        await native.page.screenshot({
          path: join(output, 'desktop-downloading.png'),
        });
        await native.context.tracing.stop({
          path: join(output, 'desktop-before-trace.zip'),
        });
      } finally {
        rmSync(hold, { force: true });
      }
      await until(async () => {
        let connected;
        try {
          connected = await connectDesktop(lab);
          const value = await connected.invoke('desktop_update_status');
          if (value.installed !== lab.next) {
            await connected.close();
            return false;
          }
          await native.close().catch(() => {});
          native = connected;
          onConnect(native);
          return true;
        } catch {
          await connected?.close().catch(() => {});
          return false;
        }
      }, 180000);
      await native.context.tracing.start({
        screenshots: true,
        snapshots: true,
      });
      await expect.poll(windowState).toEqual(expectedWindow);
      expect((await native.invoke('desktop_update_status')).policy).toBe(
        'notify',
      );
      expect(await native.invoke('server_url')).toBe(lab.baseUrl);
      const session = await native.invoke('backend_request', {
        path: '/auth/me',
        method: 'GET',
        body: null,
      });
      expect(session.status).toBe(200);
      await expect(
        native.page.getByRole('button', { name: 'Settings', exact: true }),
      ).toBeVisible();
      await native.page.screenshot({
        path: join(output, 'desktop-installed.png'),
      });
      await native.context.tracing.stop({
        path: join(output, 'desktop-after-trace.zip'),
      });
    },
  );
  return native;
}
