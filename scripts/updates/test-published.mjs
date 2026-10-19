// Deploy the latest published release from its own compose.yaml and images,
// then update it in-app to a candidate built from this commit. The published
// controller performs the update, as on every installation.
import { request as client, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { resourceScope } from '../ci-resources.mjs';
import { budgets, waitForState } from '../ci-readiness.mjs';
import { newer } from '../release-checks.mjs';
import { docker, save, repository } from './build.mjs';
import { createLab, stopLab, publisher, publishedRelease } from './lab.mjs';
import { keepsRelease } from './test-server.mjs';

const keep = process.argv.includes('--keep');
if (keep) resourceScope({ cleanupOnExit: false });
const output = resolve('test-results/updates', `${Date.now()}-published`);
mkdirSync(output, { recursive: true });
const report = { passed: false, started: new Date().toISOString() };
let lab, api;
try {
  const published = await publishedRelease();
  const next = JSON.parse(
    readFileSync(join(repository, 'package.json'), 'utf8'),
  ).version.replace(/\d+$/, (patch) => Number(patch) + 1);
  if (!published) report.skipped = 'the release feed has no release yet';
  else if (!newer(next, published.version))
    report.skipped = `published ${published.version} is not older than candidate ${next}`;
  if (report.skipped) {
    console.log(`Published release update skipped: ${report.skipped}`);
    report.passed = true;
  } else {
    report.base = published.version;
    lab = await createLab({ desktop: false, headless: true, published });
    report.lab = lab.id;
    report.candidate = lab.next;
    api = await client.newContext({
      extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
    });
    const call = async (path, method = 'GET', data) => {
      const response = await api.fetch(`${lab.baseUrl}/api/v1${path}`, {
        method,
        data,
        timeout: budgets.request,
      });
      if (!response.ok())
        throw Error(
          `${path}: HTTP ${response.status()} ${(await response.text()).slice(0, 500)}`,
        );
      return response.json();
    };
    expect((await call('/health')).version).toBe(published.version);
    await call('/setup', 'POST', {
      username: 'admin',
      password: 'update lab passphrase',
    });
    await call('/admin/product-update/policy', 'POST', {
      policy: 'notify',
      window_start: 0,
      window_end: 0,
    });
    await publisher(lab, '/control', { mode: 'candidate' });
    await call('/admin/product-update/check', 'POST');
    expect((await call('/admin/product-update')).release.version).toBe(
      lab.next,
    );
    // The one-click request: the server waits for idle, then its controller
    // prepares and activates. The server restarts in between.
    const queued = await call('/admin/product-update/install', 'POST', {
      version: lab.next,
    });
    const operation = await waitForState(
      `Published ${lab.base} updates to ${lab.next}`,
      async () => {
        const value = await call('/admin/product-update');
        const item = value.controller.items.find(({ id }) => id === queued.id);
        if (
          [
            'blocked',
            'recovery-required',
            'runtime-failure',
            'recovered',
            'restored',
            'superseded',
          ].includes(item?.stage)
        )
          throw Object.assign(Error(JSON.stringify(item)), { fatal: true });
        return { version: value.version, item };
      },
      ({ version, item }) =>
        version === lab.next && item?.stage === 'committed',
      { timeout: 2 * budgets.provision, interval: 2000 },
    );
    save(join(output, 'operation.json'), operation);
    expect(
      JSON.parse(
        docker(
          'exec',
          `${lab.id}-controller`,
          'curl',
          '-fsS',
          '--unix-socket',
          '/run/thelxinoe/controller.sock',
          'http://localhost/health',
        ),
      ).version,
    ).toBe(lab.next);
    await keepsRelease(lab, lab.next, () => call('/health'));
    report.passed = true;
  }
} catch (error) {
  report.error = error.stack ?? String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  report.finished = new Date().toISOString();
  for (const part of lab ? ['server', 'controller'] : []) {
    const logs = spawnSync('docker', ['logs', `${lab.id}-${part}`], {
      encoding: 'utf8',
      windowsHide: true,
      timeout: budgets.request,
      maxBuffer: 8 * 1024 * 1024,
    });
    writeFileSync(
      join(output, `${part}.log`),
      `${logs.stdout ?? ''}${logs.stderr ?? ''}`,
    );
  }
  await api?.dispose().catch(() => {});
  if (lab && !keep)
    await stopLab(lab).catch((error) => {
      report.cleanupError = String(error);
      report.passed = false;
      process.exitCode = 1;
    });
  save(join(output, 'result.json'), report);
  console.log(`Published release update evidence: ${output}`);
  if (keep && lab) console.log(`Retained lab: ${join(lab.root, 'lab.json')}`);
}
