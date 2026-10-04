import { expect } from '@playwright/test';

export async function nzbgetAccess({ f, page, manager, scenario, output }) {
  const prefix = '/services/nzbget';
  const rpc = (method, params = []) =>
    page.evaluate(
      async ({ method, params }) => {
        const response = await fetch('./jsonrpc', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ method, params, id: 1 }),
        });
        if (!response.ok) throw Error(`Native RPC: HTTP ${response.status}`);
        const body = await response.json();
        if (body.error) throw Error(`Native RPC ${method} failed`);
        return body.result;
      },
      { method, params },
    );

  await scenario(
    'nzbget: private root, native assets, polling, settings and NZB upload',
    async () => {
      expect(manager('nzbget').url_base).toBe('');
      expect(manager('nzbget').access_url).toBe(prefix);
      const direct = await f.context.request.get(
        `http://localhost:${f.services.nzbget.host_port}/`,
      );
      expect(direct.status()).toBe(401);
      const errors = [];
      let polls = 0;
      const observe = (response) => {
        if (!response.url().startsWith(`${f.base}${prefix}/`)) return;
        if (response.status() >= 400) errors.push(response.status());
        if (response.url().includes('/jsonrpc')) polls++;
      };
      page.on('response', observe);
      await page.goto(`${f.base}${prefix}`);
      await expect(page.locator('#ConfigTabLink')).toBeVisible({
        timeout: 30000,
      });
      expect(new URL(page.url()).pathname).toBe(`${prefix}/`);
      await expect.poll(() => polls, { timeout: 15000 }).toBeGreaterThan(2);
      // NZBGet builds Settings from its asynchronously loaded health report.
      await page.waitForFunction(() => {
        try {
          window.SystemHealth.getSection('PATHS');
          return true;
        } catch {
          return false;
        }
      });
      await page.locator('#ConfigTabLink').click();
      await expect(page.locator('#ConfigNav')).toBeVisible();
      await page.reload();
      await expect(page.locator('#ConfigTabLink')).toBeVisible({
        timeout: 30000,
      });
      const config = await rpc('loadconfig');
      const changed = config.map((option) =>
        option.Name === 'WriteLog'
          ? { ...option, Value: option.Value === 'none' ? 'rotate' : 'none' }
          : option,
      );
      expect(await rpc('saveconfig', [changed])).toBe(true);
      expect(
        (await rpc('loadconfig')).find((o) => o.Name === 'WriteLog').Value,
      ).toBe(changed.find((o) => o.Name === 'WriteLog').Value);
      expect(await rpc('saveconfig', [config])).toBe(true);
      const nzb =
        '<?xml version="1.0"?><nzb xmlns="http://www.newzbin.com/DTD/2003/nzb"><file poster="fixture" date="1" subject="gateway fixture"><groups><group>alt.test</group></groups><segments><segment bytes="1024" number="1">fixture@invalid</segment></segments></file></nzb>';
      const id = await rpc('append', [
        'gateway-fixture.nzb',
        Buffer.from(nzb).toString('base64'),
        '',
        0,
        false,
        true,
        '',
        0,
        'FORCE',
      ]);
      expect(id).toBeGreaterThan(0);
      expect(
        (await f.upstream('nzbget', 'listgroups')).some(
          (item) => item.NZBID === id,
        ),
      ).toBe(true);
      expect(await rpc('editqueue', ['GroupFinalDelete', '', [id]])).toBe(true);
      expect(
        (await f.upstream('nzbget', 'listgroups')).some(
          (item) => item.NZBID === id,
        ),
      ).toBe(false);
      await page.screenshot({
        path: `${output}/nzbget.png`,
        fullPage: true,
        mask: [page.locator('input')],
      });
      page.off('response', observe);
      expect(errors).toEqual([]);
    },
  );

  await scenario(
    'nzbget: saved credentials, administrator authorization and RPC origin checks',
    async () => {
      const url = `${f.base}${prefix}/jsonrpc`;
      const body = { method: 'version', params: [], id: 1 };
      const accepted = await f.context.request.post(url, {
        headers: {
          Origin: f.base,
          Authorization: 'Basic wrong-browser-credentials',
        },
        data: body,
      });
      expect(accepted.status()).toBe(200);
      expect((await accepted.json()).result).toBe(
        await f.upstream('nzbget', 'version'),
      );
      expect((await f.context.request.post(url, { data: body })).status()).toBe(
        403,
      );
      expect((await f.context.request.get(`${url}/version`)).status()).toBe(
        403,
      );
      expect(
        (
          await f.context.request.get(`${url}/version`, {
            headers: { Origin: f.base },
          })
        ).status(),
      ).toBe(200);
      expect(
        (
          await f.context.request.post(url, {
            headers: { Origin: 'https://foreign.invalid' },
            data: body,
          })
        ).status(),
      ).toBe(403);
      const guest = await f.browser.newContext({ ignoreHTTPSErrors: true });
      try {
        expect(
          (
            await guest.request.post(url, {
              headers: { Origin: f.base },
              data: body,
            })
          ).status(),
        ).toBe(401);
        await f.createUser({
          username: 'nzbget-viewer',
          password: 'test-only long passphrase',
          role: 'user',
        });
        await f.api(
          '/auth/login',
          'POST',
          { username: 'nzbget-viewer', password: 'test-only long passphrase' },
          guest.request,
        );
        expect(
          (
            await guest.request.post(url, {
              headers: { Origin: f.base },
              data: body,
            })
          ).status(),
        ).toBe(403);
      } finally {
        await guest.close();
      }
      const base = await f.context.request.put(
        `${f.base}/api/v1/admin/support/${manager('nzbget').id}/access`,
        {
          headers: { 'X-Thelxinoe-Client': '1' },
          data: { url_base: '/changed' },
        },
      );
      expect(base.status()).toBe(404);
    },
  );
}
