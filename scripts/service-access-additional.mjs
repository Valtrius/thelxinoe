import { expect } from '@playwright/test';

export async function additionalAccess({ f, page, manager, scenario, output }) {
  for (const kind of ['lidarr', 'prowlarr']) {
    await scenario(
      `${kind}: native settings writes keep the configured prefix`,
      async () => {
        const key = f.config(kind).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
        const host = await f.upstream(kind, 'config/host');
        const response = await f.context.request.put(
          `${f.base}/services/${kind}/api/v1/config/host/${host.id}`,
          {
            headers: { Origin: f.base, 'X-Api-Key': key },
            data: { ...host, instanceName: `Native ${kind}` },
          },
        );
        expect(response.status()).toBe(202);
        expect((await f.upstream(kind, 'config/host')).instanceName).toBe(
          `Native ${kind}`,
        );
        expect((await f.upstream(kind, 'system/status')).urlBase).toBe(
          `/services/${kind}`,
        );
      },
    );
  }
  await scenario(
    'bazarr: native settings, assets, deep links and live polling',
    async () => {
      expect(manager('bazarr').url_base).toBe('/services/bazarr');
      expect(manager('bazarr').access_url).toBe('/services/bazarr');
      const errors = [];
      const events = [];
      const observe = async (response) => {
        if (!response.url().startsWith(`${f.base}/services/bazarr/`)) return;
        if (response.status() >= 400) errors.push(response.status());
        if (
          response.url().includes('/api/socket.io/') &&
          response.request().method() === 'GET'
        ) {
          try {
            events.push(await response.text());
          } catch {
            /* navigation closes polling */
          }
        }
      };
      page.on('response', observe);
      await page.goto(`${f.base}/services/bazarr`);
      await expect(
        page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible({ timeout: 30000 });
      await page.goto(`${f.base}/services/bazarr/settings/general`);
      await page.reload();
      await expect(page.getByText('Base URL', { exact: true })).toBeVisible({
        timeout: 30000,
      });
      const key = f
        .config('bazarr')
        .match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
      const previous = await f.upstream('bazarr', 'system/settings');
      const save = (debug) =>
        f.context.request.post(
          `${f.base}/services/bazarr/api/system/settings`,
          {
            headers: { Origin: f.base, 'X-Api-Key': key },
            form: { 'settings-general-debug': String(debug) },
          },
        );
      expect((await save(!previous.general.debug)).status()).toBe(204);
      expect(
        (await f.upstream('bazarr', 'system/settings')).general.debug,
      ).toBe(!previous.general.debug);
      await expect
        .poll(() => events.some((body) => body.includes('settings')), {
          timeout: 30000,
        })
        .toBe(true);
      expect((await save(previous.general.debug)).status()).toBe(204);
      // Saving native settings reloads Bazarr's shared configuration. Both
      // automatic manager connections must survive that reload.
      expect(
        (await f.upstream('bazarr', 'system/settings')).general,
      ).toMatchObject({
        use_radarr: true,
        use_sonarr: true,
      });
      await page.screenshot({
        path: `${output}/bazarr.png`,
        fullPage: true,
        mask: [page.locator('input')],
      });
      expect(errors).toEqual([]);
      page.off('response', observe);
      expect(
        (await f.api(`/admin/support/${manager('bazarr').id}`)).movies,
      ).toEqual([]);
    },
  );
  for (const kind of ['lidarr', 'prowlarr', 'bazarr', 'nzbget']) {
    await scenario(
      `${kind}: desktop handoff is scoped and native keys do not bypass the gateway`,
      async () => {
        const client = await f.browser.newContext({ ignoreHTTPSErrors: true });
        const browser = await f.browser.newContext({ ignoreHTTPSErrors: true });
        try {
          const login = await f.api(
            '/auth/login',
            'POST',
            {
              username: 'admin',
              password: 'test-only long passphrase',
              transport: 'device',
              device_name: `Native ${kind}`,
            },
            client.request,
          );
          const headers = {
            'X-Thelxinoe-Client': '1',
            Authorization: `Bearer ${login.token}`,
          };
          const ticketResponse = await client.request.post(
            `${f.base}/api/v1/admin/managers/${manager(kind).id}/access-ticket`,
            { headers },
          );
          expect(ticketResponse.status()).toBe(200);
          const ticket = await ticketResponse.json();
          const tab = await browser.newPage();
          await tab.goto(`${f.base}${ticket.path}#${ticket.ticket}`);
          await expect(
            kind === 'nzbget'
              ? tab.locator('#ConfigTabLink')
              : tab.getByRole('link', { name: 'Settings', exact: true }),
          ).toBeVisible({ timeout: 30000 });
          expect(
            new URL(tab.url()).pathname.startsWith(`/services/${kind}/`),
          ).toBe(true);
          expect(
            (await browser.request.get(`${f.base}/api/v1/auth/me`)).status(),
          ).toBe(401);
          expect(
            (
              await browser.request.get(
                `${f.base}/services/radarr/api/v3/system/status`,
              )
            ).status(),
          ).toBe(401);
          const cookies = await browser.cookies();
          expect(
            cookies.some(
              (c) => c.path === `/services/${kind}/` && c.httpOnly && c.secure,
            ),
          ).toBe(true);
          await client.request.post(`${f.base}/api/v1/auth/logout`, {
            headers,
          });
          expect(
            (
              await browser.request.get(`${f.base}/services/${kind}/`, {
                headers: { 'X-Api-Key': 'native-key-cannot-authorize' },
              })
            ).status(),
          ).toBe(401);
        } finally {
          await browser.close();
          await client.close();
        }
      },
    );
  }
}
