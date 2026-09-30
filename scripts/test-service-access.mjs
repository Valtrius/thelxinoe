import { expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { fixtureImages, resourceRecord } from './ci-resources.mjs';
import { docker, fixture } from './service-access-fixture.mjs';
import { additionalAccess } from './service-access-additional.mjs';
import { nzbgetAccess } from './service-access-nzbget.mjs';

if (!process.argv.includes('--built')) {
  const images = fixtureImages();
  process.env.THELXINOE_SERVER_IMAGE = images.server;
  process.env.THELXINOE_CONTROLLER_IMAGE = images.controller;
  resourceRecord({ images: Object.values(images), closed: false });
  for (const target of ['server', 'controller']) {
    execFileSync(
      'docker',
      ['build', '--target', target, '-t', images[target], '.'],
      { stdio: 'inherit' },
    );
  }
}
const output = 'test-results/service-access';
mkdirSync(output, { recursive: true });
const result = {
  revision: execFileSync('git', ['rev-parse', 'HEAD'], {
    encoding: 'utf8',
  }).trim(),
  command: `npm run test:service-access${process.argv.includes('--desktop') ? ' -- --desktop' : ''}`,
  scenarios: [],
  images: {},
  passed: false,
};
const f = await fixture();
async function scenario(name, run) {
  try {
    await run();
  } catch (error) {
    result.scenarios.push({ name, passed: false });
    throw error;
  }
  result.scenarios.push({ name, passed: true });
  console.log(`PASS ${name}`);
}
try {
  for (const kind of [
    'radarr',
    'sonarr',
    'lidarr',
    'prowlarr',
    'bazarr',
    'nzbget',
  ]) {
    await f.install(kind);
    result.images[kind] = docker(
      'inspect',
      '--format',
      '{{.Image}}',
      f.services[kind].container_id,
    );
  }
  // Native settings checks start after automatic setup has finished writing
  // configuration and restarting services.
  await expect
    .poll(
      async () =>
        (await f.api('/admin/service-connections')).items
          .filter((link) => link.enabled && link.state !== 'connected')
          .map(
            (link) =>
              `${link.source_kind} -> ${link.target_kind}: ${link.state}`,
          ),
      { timeout: 180000, intervals: [1000] },
    )
    .toEqual([]);
  const managers = [
    ...(await f.api('/admin/managers')).items,
    ...(await f.api('/admin/support')).items,
  ];
  const manager = (kind) => managers.find((m) => m.kind === kind);
  const page = await f.context.newPage();
  const sockets = [];
  page.on('websocket', (socket) => {
    const frames = [];
    socket.on('framereceived', ({ payload }) => frames.push(String(payload)));
    sockets.push({ socket, frames });
  });
  const key = f.config('radarr').match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  const native = (path, method = 'GET', body) =>
    page.evaluate(
      async ({ path, method, body, key }) => {
        const response = await fetch(`/services/radarr/api/v3/${path}`, {
          method,
          headers: {
            'Content-Type': 'application/json',
            'X-Api-Key': key,
          },
          body: body === undefined ? undefined : JSON.stringify(body),
        });
        const text = await response.text();
        return {
          status: response.status,
          body: text ? JSON.parse(text) : null,
        };
      },
      { path, method, body, key },
    );
  for (const kind of ['radarr', 'sonarr', 'lidarr', 'prowlarr']) {
    await scenario(
      `${kind}: private root, prefixed API and native browser`,
      async () => {
        expect(manager(kind).url_base).toBe(`/services/${kind}`);
        expect(manager(kind).access_url).toBe(`/services/${kind}`);
        const root = await f.context.request.get(
          `http://localhost:${f.services[kind].host_port}/`,
          { maxRedirects: 0 },
        );
        expect(root.status()).toBe(307);
        expect(root.headers().location).toContain(`/services/${kind}`);
        const errors = [];
        const failed = (response) => {
          if (
            response.url().startsWith(`${f.base}/services/${kind}/`) &&
            response.status() >= 400
          )
            errors.push(response.status());
        };
        page.on('response', failed);
        await page.goto(`${f.base}${manager(kind).access_url}`);
        await expect(
          page.getByRole('link', { name: 'Settings', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        await expect
          .poll(
            () =>
              sockets.some(
                ({ socket, frames }) =>
                  !socket.isClosed() &&
                  new URL(socket.url()).pathname.startsWith(
                    `/services/${kind}/`,
                  ) &&
                  frames.length > 0,
              ),
            { timeout: 20000 },
          )
          .toBe(true);
        await page.goto(`${f.base}/services/${kind}/settings/ui`);
        await page.reload();
        await expect(
          kind === 'prowlarr'
            ? page.getByRole('group', { name: 'Dates', exact: true })
            : page.getByRole('group', { name: 'Calendar', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        await page.screenshot({
          path: `${output}/${kind}.png`,
          fullPage: true,
        });
        expect(errors).toEqual([]);
        page.off('response', failed);
        if (kind === 'prowlarr')
          expect(
            (await f.api(`/admin/support/${manager(kind).id}`)).indexers,
          ).toEqual([]);
        else
          expect(
            (await f.api(`/admin/managers/${manager(kind).id}/options`))
              .profiles.length,
          ).toBeGreaterThan(0);
      },
    );
  }
  await additionalAccess({ f, page, manager, scenario, output, result });
  await nzbgetAccess({ f, page, manager, scenario, output, result });
  await scenario('native mutation and upload body', async () => {
    const host = (await native('config/host')).body;
    const saved = await native(`config/host/${host.id}`, 'PUT', {
      ...host,
      instanceName: 'Thelxinoe E2E Radarr',
    });
    expect([200, 202]).toContain(saved.status);
    expect((await native('config/host')).body.instanceName).toBe(
      'Thelxinoe E2E Radarr',
    );
    await page.goto(`${f.base}/services/radarr/settings/ui`);
    const original = await native('config/ui');
    expect(original.status).toBe(200);
    const changed = {
      ...original.body,
      firstDayOfWeek: original.body.firstDayOfWeek === 0 ? 1 : 0,
    };
    expect(
      (await native(`config/ui/${original.body.id}`, 'PUT', changed)).status,
    ).toBe(202);
    expect((await native('config/ui')).body.firstDayOfWeek).toBe(
      changed.firstDayOfWeek,
    );
    await native(`config/ui/${original.body.id}`, 'PUT', original.body);
    // A >64KB native JSON body must reach Radarr, not the application's JSON limit.
    const large = await native('tag', 'POST', { label: 'x'.repeat(70000) });
    expect(large.status).not.toBe(413);
    if (large.status < 300) await native(`tag/${large.body.id}`, 'DELETE');
  });
  await scenario(
    'SignalR delivers native changes and rejects foreign origins',
    async () => {
      const livePage = await f.context.newPage();
      const liveSockets = [];
      livePage.on('websocket', (socket) => {
        const frames = [];
        socket.on('framereceived', ({ payload }) =>
          frames.push(String(payload)),
        );
        liveSockets.push({ socket, frames });
      });
      await livePage.goto(`${f.base}/services/radarr/settings/tags`);
      const received = (frames, name) =>
        frames.some((frame) =>
          frame.split('\u001e').some((packet) => {
            if (!packet) return false;
            const message = JSON.parse(packet);
            return (
              message.target === 'receiveMessage' &&
              message.arguments?.some((event) => event.name === name)
            );
          }),
        );
      let subscribed;
      await expect
        .poll(
          () => {
            subscribed = liveSockets.find(
              ({ socket, frames }) =>
                new URL(socket.url()).pathname ===
                  '/services/radarr/signalr/messages' &&
                !socket.isClosed() &&
                received(frames, 'version'),
            );
            return Boolean(subscribed);
          },
          { timeout: 20000 },
        )
        .toBe(true);
      subscribed.frames.length = 0;
      const tag = await native('tag', 'POST', {
        label: `gateway-live-check-${Date.now()}`,
      });
      expect(tag.status).toBe(201);
      await expect
        .poll(
          () =>
            !subscribed.socket.isClosed() && received(subscribed.frames, 'tag'),
          { timeout: 20000 },
        )
        .toBe(true);
      await native(`tag/${tag.body.id}`, 'DELETE');
      const rejected = await f.context.request.get(
        `${f.base}/services/radarr/signalr/messages`,
        {
          headers: {
            Origin: 'https://evil.example',
            Upgrade: 'websocket',
            Connection: 'Upgrade',
            'Sec-WebSocket-Version': '13',
            'Sec-WebSocket-Key': 'dGhlIHNhbXBsZSBub25jZQ==',
          },
        },
      );
      expect(rejected.status()).toBe(403);
      await livePage.close();
    },
  );
  await scenario(
    'anonymous and regular users cannot use native keys to bypass access',
    async () => {
      const anonymous = await f.browser.newContext({ ignoreHTTPSErrors: true });
      const key = f.config('radarr').match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
      const denied = await anonymous.request.get(
        `${f.base}/services/radarr/api/v3/system/status`,
        { headers: { 'X-Api-Key': key }, maxRedirects: 0 },
      );
      expect(denied.status()).toBe(401);
      await f.api('/users', 'POST', {
        username: 'viewer',
        password: 'test-only viewer passphrase',
        role: 'user',
      });
      await f.api(
        '/auth/login',
        'POST',
        { username: 'viewer', password: 'test-only viewer passphrase' },
        anonymous.request,
      );
      expect(
        (
          await anonymous.request.get(`${f.base}/services/radarr/`, {
            maxRedirects: 0,
          })
        ).status(),
      ).toBe(403);
      await anonymous.close();
    },
  );
  await scenario(
    'backup download, range and multipart restore retain the base',
    async () => {
      const command = await native('command', 'POST', { name: 'Backup' });
      expect(command.status).toBe(201);
      await expect
        .poll(
          async () => (await native(`command/${command.body.id}`)).body.status,
          { timeout: 60000 },
        )
        .toBe('completed');
      const backup = (await native('system/backup')).body[0];
      const url = `${f.base}/services/radarr${backup.path}`;
      const downloaded = await f.context.request.get(url);
      expect(downloaded.status()).toBe(200);
      expect(downloaded.headers()['content-type']).toContain('zip');
      const bytes = await downloaded.body();
      expect(bytes.subarray(0, 2).toString()).toBe('PK');
      const range = await f.context.request.get(url, {
        headers: { Range: 'bytes=0-99' },
      });
      const directRange = await f.context.request.get(
        `http://localhost:${f.services.radarr.host_port}/services/radarr${backup.path}`,
        { headers: { Range: 'bytes=0-99' } },
      );
      expect(range.status()).toBe(directRange.status());
      expect(await range.body()).toEqual(await directRange.body());
      const restored = await f.context.request.post(
        `${f.base}/services/radarr/api/v3/system/backup/restore/upload`,
        {
          headers: { Origin: f.base, 'X-Api-Key': key },
          multipart: {
            file: {
              name: backup.name,
              mimeType: 'application/zip',
              buffer: bytes,
            },
          },
        },
      );
      expect(restored.status()).toBe(200);
      expect((await restored.json()).restartRequired).toBe(true);
      await native('command', 'POST', { name: 'Restart' });
      await expect
        .poll(
          async () => {
            try {
              return (await f.upstream('radarr', 'system/status')).urlBase;
            } catch {
              return '';
            }
          },
          { timeout: 90000, intervals: [2000] },
        )
        .toBe('/services/radarr');
      await page.goto(`${f.base}/services/radarr/settings/ui`);
      await expect(
        page.getByRole('group', { name: 'Calendar', exact: true }),
      ).toBeVisible({ timeout: 30000 });
    },
  );
  await scenario(
    'cross-origin mutations and invalid paths fail closed',
    async () => {
      for (const headers of [
        { Origin: 'https://evil.example' },
        { 'Sec-Fetch-Site': 'cross-site' },
        {},
      ]) {
        const response = await f.context.request.post(
          `${f.base}/services/radarr/api/v3/tag`,
          { headers, data: { label: 'forbidden' } },
        );
        expect(response.status()).toBe(403);
      }
      for (const url_base of [
        '/api',
        '/assets',
        '/seerr-bootstrap',
        '//evil.example',
        '/x/../api',
        '/services/sonarr',
        '/a%2fb',
      ]) {
        const response = await f.context.request.put(
          `${f.base}/api/v1/admin/managers/${manager('radarr').id}/access`,
          {
            headers: { 'X-Thelxinoe-Client': '1' },
            data: { url_base },
          },
        );
        expect(response.status()).toBe(404);
      }
    },
  );
  await scenario(
    'desktop handoff is single use, scoped, and revoked with its parent',
    async () => {
      const client = await f.browser.newContext({ ignoreHTTPSErrors: true });
      const login = await f.api(
        '/auth/login',
        'POST',
        {
          username: 'admin',
          password: 'test-only long passphrase',
          transport: 'device',
          device_name: 'E2E desktop',
        },
        client.request,
      );
      const ticketResponse = await client.request.post(
        `${f.base}/api/v1/admin/managers/${manager('radarr').id}/access-ticket`,
        {
          headers: {
            'X-Thelxinoe-Client': '1',
            Authorization: `Bearer ${login.token}`,
          },
        },
      );
      expect(ticketResponse.status()).toBe(200);
      const ticket = await ticketResponse.json();
      const browser = await f.browser.newContext({ ignoreHTTPSErrors: true });
      await browser.addCookies([
        {
          name: 'thelxinoe_session',
          value: 'expired-session'.padEnd(64, '0'),
          url: f.base,
          secure: true,
          httpOnly: true,
          sameSite: 'Strict',
        },
      ]);
      const tab = await browser.newPage();
      const browserSockets = [];
      tab.on('websocket', (socket) => browserSockets.push(socket));
      await tab.goto(`${f.base}${ticket.path}#${ticket.ticket}`);
      await expect(
        tab.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible({ timeout: 30000 });
      await expect
        .poll(() => browserSockets.some((socket) => !socket.isClosed()), {
          timeout: 20000,
        })
        .toBe(true);
      expect(new URL(tab.url()).hash).toBe('');
      expect(
        (await browser.request.get(`${f.base}/api/v1/auth/me`)).status(),
      ).toBe(401);
      expect(
        (
          await browser.request.get(
            `${f.base}/services/sonarr/api/v3/system/status`,
          )
        ).status(),
      ).toBe(401);
      const replay = await browser.request.post(`${f.base}${ticket.path}`, {
        headers: { Origin: f.base },
        data: { ticket: ticket.ticket },
      });
      expect(replay.status()).toBe(401);
      const issue = async () =>
        (
          await client.request.post(
            `${f.base}/api/v1/admin/managers/${manager('radarr').id}/access-ticket`,
            {
              headers: {
                'X-Thelxinoe-Client': '1',
                Authorization: `Bearer ${login.token}`,
              },
            },
          )
        ).json();
      const wrongService = await issue();
      expect(
        (
          await browser.request.post(
            `${f.base}/service-access/${manager('sonarr').id}`,
            {
              headers: { Origin: f.base },
              data: { ticket: wrongService.ticket },
            },
          )
        ).status(),
      ).toBe(401);
      const expired = await issue();
      await new Promise((done) => setTimeout(done, 31000));
      expect(
        (
          await browser.request.post(`${f.base}${expired.path}`, {
            headers: { Origin: f.base },
            data: { ticket: expired.ticket },
          })
        ).status(),
      ).toBe(401);
      await client.request.post(`${f.base}/api/v1/auth/logout`, {
        headers: {
          'X-Thelxinoe-Client': '1',
          Authorization: `Bearer ${login.token}`,
        },
      });
      expect(
        (
          await browser.request.get(
            `${f.base}/services/radarr/api/v3/system/status`,
          )
        ).status(),
      ).toBe(401);
      await expect
        .poll(() => browserSockets.every((socket) => socket.isClosed()), {
          timeout: 15000,
        })
        .toBe(true);
      await browser.close();
      await client.close();
    },
  );
  const { ownershipAccess, retireService, attachedConflicts } =
    await import('./service-access-ownership.mjs');
  await ownershipAccess({ f, page, scenario, output });
  await retireService(f, 'radarr');
  await attachedConflicts({ f, page, scenario, output });
  await scenario(
    'native headers and cookies cannot escape their service scope',
    async () => {
      await f.peer();
      const range = await f.context.request.get(
        `${f.base}/services/radarr/range`,
        { headers: { Range: 'bytes=0-4' } },
      );
      expect(range.status()).toBe(206);
      expect(range.headers()['content-range']).toBe('bytes 0-4/11');
      expect(range.headers()['content-disposition']).toContain('fixture.bin');
      expect(await range.text()).toBe('range');
      const headers = await (
        await f.context.request.get(`${f.base}/services/radarr/headers`, {
          headers: {
            Authorization: 'Bearer must-not-reach-native',
            'X-Thelxinoe-Client': '1',
            'X-Forwarded-Host': 'evil.example',
          },
        })
      ).json();
      expect(
        'authorization' in headers ||
          'cookie' in headers ||
          'x-thelxinoe-client' in headers,
      ).toBe(false);
      expect(headers['x-forwarded-host']).toBe(new URL(f.base).host);
      const cookies = await f.context.request.get(
        `${f.base}/services/radarr/cookies`,
      );
      expect(cookies.headers()['service-worker-allowed']).toBeUndefined();
      expect((await f.api('/auth/me')).user.role).toBe('admin');
      const nativeCookies = (await f.context.cookies()).filter(
        (c) =>
          c.name.startsWith('thelxinoe_native_') &&
          c.path === '/services/radarr/',
      );
      expect(nativeCookies).toHaveLength(2);
      expect(
        nativeCookies.every(
          (c) =>
            c.domain === 'localhost' && c.secure && c.sameSite === 'Strict',
        ),
      ).toBe(true);
      const restored = await (
        await f.context.request.get(`${f.base}/services/radarr/headers`)
      ).json();
      expect(restored.cookie).toContain('native=fixture');
      expect(restored.cookie).toContain('thelxinoe_session=overwrite');
      expect(
        (
          await f.context.request.get(`${f.base}/services/radarr/redirect`, {
            maxRedirects: 0,
          })
        ).status(),
      ).toBe(502);
      expect(
        (
          await f.context.request.get(`${f.base}/services/radarr/headers`, {
            headers: { 'Service-Worker': 'script' },
          })
        ).status(),
      ).toBe(403);
    },
  );
  await scenario('logout closes an active streamed HTTP response', async () => {
    const client = await f.browser.newContext({ ignoreHTTPSErrors: true });
    await f.api(
      '/auth/login',
      'POST',
      { username: 'admin', password: 'test-only long passphrase' },
      client.request,
    );
    const tab = await client.newPage();
    const activity = async () =>
      (
        await f.context.request.get(`${f.base}/services/radarr/activity`)
      ).json();
    result.logout_stream = {};
    try {
      await tab.goto(f.base);
      await tab.evaluate(() => {
        window.streamMessages = 0;
        window.streamClosed = false;
        window.pendingClosed = false;
        void fetch('/services/radarr/pending', {
          method: 'POST',
          body: 'fixture',
        })
          .finally(() => {
            window.pendingClosed = true;
          })
          .catch(() => {});
        const stream = new EventSource('/services/radarr/stream');
        stream.onmessage = ({ data }) => {
          if (data === 'fixture') window.streamMessages++;
        };
        stream.onerror = () => {
          window.streamClosed = true;
          stream.close();
        };
      });
      await expect
        .poll(() => tab.evaluate(() => window.streamMessages), {
          timeout: 30000,
        })
        .toBeGreaterThan(2);
      expect(await tab.evaluate(() => window.streamClosed)).toBe(false);
      expect(await tab.evaluate(() => window.pendingClosed)).toBe(false);
      await expect.poll(activity, { timeout: 30000 }).toEqual({
        streams: 1,
        pending: 1,
      });
      result.logout_stream.before_logout = await activity();
      await f.api('/auth/logout', 'POST', undefined, client.request);
      await expect
        .poll(() => tab.evaluate(() => window.streamClosed), {
          timeout: 15000,
        })
        .toBe(true);
      await expect
        .poll(() => tab.evaluate(() => window.pendingClosed), {
          timeout: 15000,
        })
        .toBe(true);
      await expect.poll(activity, { timeout: 15000 }).toEqual({
        streams: 0,
        pending: 0,
      });
      result.logout_stream.after_logout = await activity();
    } finally {
      result.logout_stream.browser = await tab
        .evaluate(() => ({
          messages: window.streamMessages,
          stream_closed: window.streamClosed,
          pending_closed: window.pendingClosed,
        }))
        .catch(() => null);
      await client.close();
    }
  });
  await scenario(
    'current administrator role is checked again on each native request',
    async () => {
      await f.api('/users', 'POST', {
        username: 'remaining-admin',
        password: 'test-only remaining admin passphrase',
        role: 'admin',
      });
      f.compose(
        'exec',
        '-T',
        'server',
        'python',
        '-c',
        "import sqlite3; c=sqlite3.connect('/var/lib/thelxinoe/thelxinoe.sqlite3'); c.execute(\"UPDATE users SET role='user' WHERE username='admin'\"); c.commit()",
      );
      expect(
        (
          await f.context.request.get(`${f.base}/services/radarr/headers`)
        ).status(),
      ).toBe(403);
    },
  );
  await f.close();
  await scenario(
    'local HTTP access uses the same native interface',
    async () => {
      const local = await fixture({ scheme: 'http' });
      try {
        await local.install('radarr');
        const tab = await local.context.newPage();
        await tab.goto(`${local.base}/services/radarr`);
        await expect(
          tab.getByRole('link', { name: 'Settings', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        const session = (await local.context.cookies()).find(
          (cookie) => cookie.name === 'thelxinoe_session',
        );
        expect(session.secure).toBe(false);
        await tab.screenshot({
          path: `${output}/local-http.png`,
          fullPage: true,
        });
        if (process.argv.includes('--desktop')) {
          const { desktopLaunch } =
            await import('./service-access-desktop.mjs');
          for (const kind of [
            'sonarr',
            'lidarr',
            'prowlarr',
            'bazarr',
            'nzbget',
          ])
            await local.install(kind);
          await scenario(
            'Windows keyring and real default-browser launch for all six services',
            () =>
              desktopLaunch(local, output, [
                'radarr',
                'sonarr',
                'lidarr',
                'prowlarr',
                'bazarr',
                'nzbget',
              ]),
          );
        }
      } finally {
        await local.close();
      }
    },
  );
  result.passed = true;
} finally {
  writeFileSync(`${output}/result.json`, JSON.stringify(result, null, 2));
  if (!result.passed && process.env.THELXINOE_KEEP_FAILED_FIXTURE === '1') {
    writeFileSync(
      `${output}/fixture.json`,
      JSON.stringify({
        project: f.project,
        root: f.root,
        base: f.base,
        services: f.services,
      }),
    );
    await f.browser.close();
  } else await f.close();
}
