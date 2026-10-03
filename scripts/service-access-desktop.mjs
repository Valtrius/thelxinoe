// Windows-only real Tauri command + OS browser handoff. Each build uses a fresh
// app identifier, so the fixture cannot read or replace a personal keyring entry.
import { chromium, expect } from '@playwright/test';
import { spawn, execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { freePort } from './service-access-fixture.mjs';

export async function desktopLaunch(fixture, output, kinds = ['radarr']) {
  if (process.platform !== 'win32')
    throw Error('Desktop service access requires Windows');
  const report = {
    passed: false,
    services: kinds,
    scenario: 'Windows keyring and real default-browser launch',
  };
  writeFileSync(
    `${output}/desktop-result.json`,
    JSON.stringify(report, null, 2),
  );
  const identifier = `app.thelxinoe.serviceaccess${Date.now()}`;
  const config = resolve(`${fixture.root}/desktop.json`);
  writeFileSync(
    config,
    JSON.stringify({
      identifier,
      app: {
        windows: [
          {
            title: 'Thelxinoe service access test',
            width: 1360,
            height: 900,
            visible: false,
          },
        ],
      },
    }),
  );
  execFileSync(
    process.execPath,
    [
      resolve('node_modules/@tauri-apps/cli/tauri.js'),
      'build',
      '--debug',
      '--no-bundle',
      '--config',
      config,
    ],
    { cwd: 'apps/desktop', stdio: 'inherit' },
  );
  const port = await freePort();
  const app = spawn(resolve('target/debug/thelxinoe-desktop.exe'), [], {
    windowsHide: true,
    stdio: 'ignore',
    env: {
      ...process.env,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    },
  });
  let browser, page;
  try {
    await expect
      .poll(
        async () => {
          try {
            browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
            return true;
          } catch {
            return false;
          }
        },
        { timeout: 60000, intervals: [1000] },
      )
      .toBe(true);
    page = browser.contexts()[0].pages()[0];
    await page.getByLabel('Server address').fill(fixture.base);
    await page.getByLabel('Username', { exact: true }).fill('admin');
    await page
      .getByLabel('Password', { exact: true })
      .fill('test-only long passphrase');
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await page
      .getByRole('link', { name: 'Media services', exact: true })
      .click();
    const source = await page.evaluate(() => ({
      cookie: document.cookie,
      local: Object.keys(localStorage),
      session: Object.keys(sessionStorage),
    }));
    expect(source.cookie).not.toContain('thelxinoe_session');
    expect(
      [...source.local, ...source.session].some((key) =>
        /token|session|password/i.test(key),
      ),
    ).toBe(false);
    for (const [index, kind] of kinds.entries()) {
      const label =
        kind === 'nzbget' ? 'NZBGet' : kind[0].toUpperCase() + kind.slice(1);
      await page
        .getByRole('navigation', { name: 'Select service' })
        .getByRole('link', { name: label, exact: true })
        .click();
      await page
        .getByRole('link', { name: `Open ${label}`, exact: true })
        .click();
      // The OS browser consumes the fragment into a scoped service grant. The
      // desktop renderer never receives the bearer or the launch credential.
      await expect
        .poll(
          () =>
            Number(
              fixture.compose(
                'exec',
                '-T',
                'server',
                'python',
                '-c',
                "import sqlite3; c=sqlite3.connect('/var/lib/thelxinoe/thelxinoe.sqlite3'); print(c.execute(\"SELECT COUNT(*) FROM playback_grants WHERE resource LIKE 'service-browser:%'\").fetchone()[0])",
              ),
            ),
          { timeout: 30000, intervals: [1000] },
        )
        .toBe(index + 1);
      await page.screenshot({ path: `${output}/desktop-${kind}.png` });
    }
    report.passed = true;
    writeFileSync(
      `${output}/desktop-result.json`,
      JSON.stringify(report, null, 2),
    );
  } catch (error) {
    if (page)
      await page
        .screenshot({ path: `${output}/desktop-failure.png` })
        .catch(() => {});
    throw error;
  } finally {
    if (page)
      await page
        .evaluate(async (value) => {
          await window.__TAURI_INTERNALS__.invoke('change_server', { value });
        }, fixture.base)
        .catch(() => {});
    if (browser) await browser.close();
    app.kill();
  }
}
