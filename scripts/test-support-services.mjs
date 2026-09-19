// Uses only isolated acquisition service credentials; never logs them.
import { chromium, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const browser = await chromium.launch();
const context = await browser.newContext({ ignoreHTTPSErrors: true });
const page = await context.newPage();
const base = 'https://localhost:23443';
const headers = { 'X-Thelxinoe-Client': '1' };
async function api(path, method = 'GET', data) {
  const r = await context.request.fetch(`${base}/api/v1${path}`, {
    method,
    data,
    headers,
  });
  if (!r.ok())
    throw Error(
      `${path}: HTTP ${r.status()} ${(await r.json()).error?.message}`,
    );
  return r.json();
}
const results = [];
try {
  await api('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  await page.goto(base);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const containers = (await api('/admin/managers/containers')).items;
  for (const [kind, port] of [
    ['bazarr', 26767],
    ['prowlarr', 29696],
    ['nzbget', 26789],
  ]) {
    let username = '',
      secret;
    if (kind === 'bazarr')
      secret = readFileSync(
        '.local/acquisition/bazarr/config/config.yaml',
        'utf8',
      ).match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
    else if (kind === 'prowlarr')
      secret = readFileSync(
        '.local/acquisition/prowlarr/config.xml',
        'utf8',
      ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
    else {
      const config = readFileSync(
        '.local/acquisition/nzbget/nzbget.conf',
        'utf8',
      );
      username = config.match(/^ControlUsername=(.*)$/m)[1].trim();
      secret = config.match(/^ControlPassword=(.*)$/m)[1].trim();
    }
    const container = containers.find((c) =>
      c.names.some((n) => n.includes(`acquisition-${kind}-`)),
    );
    if (!container) throw Error(`Missing fixture ${kind}`);
    await page
      .getByLabel('Service name', { exact: true })
      .fill(`Fixture ${kind}`);
    await page.getByLabel('Service type').selectOption(kind);
    await page.getByLabel('Service container').selectOption(container.id);
    if (kind === 'nzbget')
      await page.getByLabel('NZBGet username', { exact: true }).fill(username);
    await page
      .getByLabel(kind === 'nzbget' ? 'NZBGet password' : 'Service API key', {
        exact: true,
      })
      .fill(secret);
    await page
      .getByLabel('Native service UI address', { exact: true })
      .fill(`http://localhost:${port}`);
    const registering = page.waitForResponse(
      (r) =>
        r.url().endsWith('/api/v1/admin/support') &&
        r.request().method() === 'POST',
    );
    await page
      .getByRole('button', { name: 'Connect support service', exact: true })
      .click();
    const response = await registering;
    if (!response.ok())
      throw Error(`${kind} registration: HTTP ${response.status()}`);
    await expect(
      page.getByRole('button', {
        name: `Refresh Fixture ${kind}`,
        exact: true,
      }),
    ).toBeVisible();
    const list = await api('/admin/support');
    expect(JSON.stringify(list)).not.toContain(secret);
    const service = list.items.find((s) => s.kind === kind);
    let status = await api(`/admin/support/${service.id}`);
    if (kind === 'nzbget') {
      const paused = status.paused,
        limit = status.limit;
      await api(`/admin/support/${service.id}`, 'POST', {
        action: 'pause_all',
      });
      expect((await api(`/admin/support/${service.id}`)).paused).toBe(true);
      await api(`/admin/support/${service.id}`, 'POST', {
        action: 'rate',
        value: 128,
      });
      expect((await api(`/admin/support/${service.id}`)).limit).toBe(
        128 * 1024,
      );
      const nzb = `<?xml version="1.0"?><nzb xmlns="http://www.newzbin.com/DTD/2003/nzb"><file poster="fixture" date="${Math.floor(Date.now() / 1000)}" subject="fixture.bin"><groups><group>alt.test</group></groups><segments><segment bytes="1024" number="1">fixture@thelxinoe.invalid</segment></segments></file></nzb>`;
      const rpc = await fetch('http://127.0.0.1:26789/jsonrpc', {
        method: 'POST',
        headers: {
          Authorization: `Basic ${Buffer.from(`${username}:${secret}`).toString('base64')}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          method: 'append',
          params: [
            `fixture-${Date.now()}.nzb`,
            Buffer.from(nzb).toString('base64'),
            'fixture',
            0,
            false,
            true,
            '',
            0,
            'FORCE',
            false,
            [],
          ],
          id: 1,
        }),
      });
      const added = await rpc.json();
      if (added.error || !(added.result > 0))
        throw Error('NZBGet fixture append failed');
      const item_id = added.result;
      await expect
        .poll(async () =>
          (await api(`/admin/support/${service.id}`)).queue.some(
            (r) => r.id === item_id,
          ),
        )
        .toBe(true);
      for (const action of ['resume', 'pause', 'remove'])
        await api(`/admin/support/${service.id}`, 'POST', { action, item_id });
      await expect
        .poll(async () =>
          (await api(`/admin/support/${service.id}`)).queue.some(
            (r) => r.id === item_id,
          ),
        )
        .toBe(false);
      await api(`/admin/support/${service.id}`, 'POST', {
        action: 'rate',
        value: Math.floor(limit / 1024),
      });
      await api(`/admin/support/${service.id}`, 'POST', {
        action: paused ? 'pause_all' : 'resume_all',
      });
      status = await api(`/admin/support/${service.id}`);
    }
    results.push({
      kind,
      version: service.version,
      healthy: true,
      fields: Object.keys(status),
    });
    console.log(JSON.stringify(results.at(-1)));
  }
  await page.screenshot({
    path: '.local/support-services.png',
    fullPage: true,
  });
  writeFileSync(
    '.local/support-services-result.json',
    JSON.stringify(results, null, 2),
  );
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
} finally {
  await browser.close();
}
