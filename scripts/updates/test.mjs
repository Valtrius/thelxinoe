import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, readFileSync, copyFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { join, resolve } from 'node:path';
import {
  createLab,
  readLab,
  stopLab,
  publisher,
  desktopInstallation,
} from './lab.mjs';
import { docker, save } from './build.mjs';
import { serverScenarios } from './test-server.mjs';
import { desktopScenarios, connectDesktop } from './test-desktop.mjs';

const arguments_ = process.argv.slice(2);
const reuse = arguments_.indexOf('--lab');
const output = resolve('test-results/updates', String(Date.now()));
mkdirSync(output, { recursive: true });
const report = {
  passed: false,
  started: new Date().toISOString(),
  scenarios: [],
};
let lab, browser, context, native, unrelatedDesktop;
try {
  if (process.platform === 'win32' && !arguments_.includes('--server-only')) {
    // NSIS used to kill all processes with the production executable name.
    // A harmless long-lived process makes that installer regression observable.
    const executable = join(output, 'thelxinoe-desktop.exe');
    copyFileSync(join(process.env.SystemRoot, 'System32/ping.exe'), executable);
    unrelatedDesktop = spawn(executable, ['-t', '127.0.0.1'], {
      windowsHide: true,
      stdio: 'ignore',
    });
    await once(unrelatedDesktop, 'spawn');
  }
  lab =
    reuse >= 0
      ? readLab(arguments_[reuse + 1])
      : await createLab({
          server: !arguments_.includes('--desktop-only'),
          desktop:
            process.platform === 'win32' &&
            !arguments_.includes('--server-only'),
          headless: true,
        });
  report.lab = lab.id;
  if (lab.desktop) {
    expect(
      unrelatedDesktop.exitCode,
      'Installing the lab closed another desktop',
    ).toBeNull();
    report.installed = desktopInstallation(lab);
    expect(report.installed).toEqual({ registered: true, executable: true });
  }
  browser = await chromium.launch();
  context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  await context.tracing.start({ screenshots: true, snapshots: true });
  const credentials = { username: 'admin', password: 'update lab passphrase' };
  const request = (path, method = 'GET', data, client = context.request) =>
    client.fetch(lab.baseUrl + '/api/v1' + path, {
      method,
      data,
      headers: { 'X-Thelxinoe-Client': '1' },
      timeout: 10000,
    });
  const api = async (...args) => {
    const response = await request(...args);
    if (!response.ok())
      throw Error(
        `${args[0]}: HTTP ${response.status()} ${(await response.text()).slice(0, 500)}`,
      );
    return response.json();
  };
  if ((await api('/setup')).setup_required)
    await api('/setup', 'POST', credentials);
  else await api('/auth/login', 'POST', credentials);
  const page = await context.newPage();
  await page.goto(lab.baseUrl);
  await expect(
    page.getByRole('button', { name: 'Settings', exact: true }),
  ).toBeVisible();
  if (lab.desktop) {
    native = await connectDesktop(lab);
    await native.invoke('desktop_update_policy', { policy: 'notify' });
    await native.invoke('change_server', { value: lab.baseUrl });
    expect(
      (
        await native.invoke('backend_request', {
          path: '/auth/login',
          method: 'POST',
          body: credentials,
        })
      ).status,
    ).toBe(200);
    await native.page.reload();
  }
  const scenario = async (name, fn) => {
    console.log(`Update scenario: ${name}`);
    const item = { name, passed: false };
    report.scenarios.push(item);
    save(join(output, 'result.json'), report);
    try {
      await fn();
      item.passed = true;
    } catch (error) {
      item.error = String(error);
      throw error;
    } finally {
      save(join(output, 'result.json'), report);
    }
  };
  const mode = (value) => publisher(lab, '/control', { mode: value });
  if (lab.server)
    await serverScenarios({
      lab,
      page,
      context,
      api,
      request,
      native,
      scenario,
      mode,
      output,
    });
  if (lab.desktop)
    native = await desktopScenarios({
      lab,
      native,
      scenario,
      mode,
      output,
      credentials,
    });
  await mode('base');
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  report.finished = new Date().toISOString();
  if (lab?.server) {
    for (const component of ['server', 'controller']) {
      try {
        writeFileSync(
          join(output, `${component}.log`),
          docker('logs', `${lab.id}-${component}`),
        );
      } catch {
        /* Container may have been replaced. */
      }
    }
    try {
      const state = docker(
        'exec',
        `${lab.id}-controller`,
        'curl',
        '-fsS',
        '--unix-socket',
        '/run/thelxinoe/controller.sock',
        'http://localhost/stack/product',
      );
      writeFileSync(join(output, 'operations.json'), state);
    } catch {
      /* Retain the existing test failure. */
    }
  }
  for (const name of ['publisher', 'server', 'desktop']) {
    try {
      writeFileSync(
        join(output, `${name}-process.log`),
        readFileSync(join(lab.root, `${name}.log`)),
      );
    } catch {
      /* Optional process. */
    }
  }
  await context?.tracing
    .stop({ path: join(output, 'web-trace.zip') })
    .catch(() => {});
  if (!report.passed)
    await native?.page
      .screenshot({ path: join(output, 'desktop-failure.png') })
      .catch(() => {});
  await native?.context.tracing
    .stop({ path: join(output, 'desktop-failure-trace.zip') })
    .catch(() => {});
  await browser?.close().catch(() => {});
  await native?.close().catch(() => {});
  if (lab && !arguments_.includes('--keep'))
    await (async () => {
      await stopLab(lab);
      if (lab.desktop) {
        report.uninstalled = desktopInstallation(lab);
        expect(report.uninstalled).toEqual({
          registered: false,
          executable: false,
        });
        await stopLab(lab);
      }
    })().catch((error) => {
      report.cleanupError = String(error);
      report.passed = false;
      process.exitCode = 1;
    });
  if (unrelatedDesktop) {
    report.unrelatedDesktopSurvived = unrelatedDesktop.exitCode === null;
    if (!report.unrelatedDesktopSurvived) {
      report.passed = false;
      report.isolationError =
        'A lab installer closed an unrelated desktop process';
      process.exitCode = 1;
    }
    unrelatedDesktop.kill();
  }
  save(join(output, 'result.json'), report);
  console.log(`Update evidence: ${output}`);
  if (arguments_.includes('--keep') && lab)
    console.log(`Retained lab: ${join(lab.root, 'lab.json')}`);
}
