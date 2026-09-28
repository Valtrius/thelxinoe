import { chromium, expect } from '@playwright/test';
import { createServer } from 'node:http';
import { join } from 'node:path';
import { until } from './lab.mjs';

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
}) {
  const idle = () =>
    until(
      async () =>
        !['checking', 'downloading', 'installing'].includes(
          (await native.invoke('desktop_update_status')).phase,
        ),
    );
  const check = async () => {
    await idle();
    return native.invoke('desktop_update_check');
  };
  const login = async () => {
    await native.invoke('change_server', { value: lab.baseUrl });
    const response = await native.invoke('backend_request', {
      path: '/auth/login',
      method: 'POST',
      body: credentials,
    });
    expect(response.status).toBe(200);
    expect(response.body.token).toBeUndefined();
    await native.page.reload();
  };
  await native.context.tracing.start({ screenshots: true, snapshots: true });
  await scenario(
    'Desktop discovers publisher releases before login with an unavailable server',
    async () => {
      await mode('candidate');
      await native.invoke('change_server', { value: 'http://127.0.0.1:1' });
      await native.page.reload();
      const value = await check();
      expect(value.installed).toBe(lab.base);
      expect(value.release.version).toBe(lab.next);
      await expect(
        native.page.getByRole('button', {
          name: 'View desktop update',
          exact: true,
        }),
      ).toBeVisible();
      await native.page
        .getByRole('button', { name: 'View desktop update', exact: true })
        .click();
      await expect(native.page.getByRole('dialog')).toContainText(
        `Version ${lab.next}`,
      );
      await native.page.screenshot({
        path: join(output, 'desktop-offline-notice.png'),
      });
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
    'Automatic downloads a verified update and waits for explicit installation',
    async () => {
      await mode('base');
      await check();
      expect((await native.invoke('desktop_update_status')).release).toBeNull();
      await mode('candidate');
      await check();
      await native.invoke('desktop_update_policy', { policy: 'automatic' });
      await expect
        .poll(
          async () => (await native.invoke('desktop_update_status')).phase,
          { timeout: 90000 },
        )
        .toBe('ready');
      expect((await native.invoke('desktop_update_status')).installed).toBe(
        lab.base,
      );
      await native.invoke('desktop_update_policy', { policy: 'notify' });
    },
  );
  await scenario(
    'Incompatible connected server blocks installation while discovery remains available',
    async () => {
      const incompatible = createServer((request, response) => {
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
        await native.invoke('change_server', {
          value: `http://127.0.0.1:${incompatible.address().port}`,
        });
        await native.page.reload();
        await expect(
          native.page.getByRole('heading', { name: 'Update required' }),
        ).toBeVisible();
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
        await new Promise((done) => incompatible.close(done));
      }
    },
  );
  await scenario(
    'Real signed installer updates and relaunches with credentials and settings preserved',
    async () => {
      await login();
      await native.invoke('desktop_update_policy', { policy: 'automatic' });
      await native.page
        .getByRole('button', { name: 'View desktop update', exact: true })
        .click();
      const dialog = native.page.getByRole('dialog');
      await expect(
        dialog.getByRole('button', {
          name: 'Restart and install desktop update',
          exact: true,
        }),
      ).toBeVisible();
      await native.page.screenshot({ path: join(output, 'desktop-ready.png') });
      await native.context.tracing.stop({
        path: join(output, 'desktop-before-trace.zip'),
      });
      await dialog
        .getByRole('button', {
          name: 'Restart and install desktop update',
          exact: true,
        })
        .click()
        .catch(() => {});
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
      expect((await native.invoke('desktop_update_status')).policy).toBe(
        'automatic',
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
