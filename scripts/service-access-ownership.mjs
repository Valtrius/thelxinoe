import { expect } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { docker } from './service-access-fixture.mjs';

export async function retireService(f, kind) {
  if (['radarr', 'sonarr', 'lidarr'].includes(kind)) {
    await expect
      .poll(
        async () =>
          (await f.upstream(kind, 'command')).filter((c) =>
            ['queued', 'started'].includes(c.status),
          ).length,
        { timeout: 90000 },
      )
      .toBe(0);
  }
  const service = f.services[kind];
  await f.api(`/admin/stack/${service.id}/action`, 'POST', { action: 'stop' });
  docker('rm', service.container_id);
  await f.api(`/admin/stack/${service.id}/action`, 'POST', {
    action: 'retire',
  });
}

export async function ownershipAccess({ f, page, scenario, output }) {
  for (const kind of [
    'radarr',
    'sonarr',
    'lidarr',
    'prowlarr',
    'bazarr',
    'nzbget',
  ]) {
    await scenario(
      `${kind}: attachment preserves configuration and adoption sets the fixed prefix`,
      async () => {
        await page.goto(f.base);
        const image = f.services[kind].image;
        await retireService(f, kind);
        if (kind === 'radarr') {
          // Finish retirement before reconnecting the retained links to the
          // replacement. Preserve Bazarr's settings so ownership still matches.
          await expect
            .poll(
              async () =>
                (await f.api('/admin/service-connections')).items
                  .filter(
                    (link) =>
                      link.target_kind === kind &&
                      link.state !== 'disconnected',
                  )
                  .map((link) => `${link.source_kind}: ${link.state}`),
              { timeout: 180000, intervals: [1000] },
            )
            .toEqual([]);
        }
        // Include empty, custom and canonical existing prefixes.
        const oldBase = {
          radarr: '',
          sonarr: '/old-sonarr',
          lidarr: '/services/lidarr',
          prowlarr: '/old-prowlarr',
          bazarr: '/old-bazarr',
          nzbget: '',
        }[kind];
        const attached = await f.attach(kind, oldBase, image);
        if (['bazarr', 'nzbget'].includes(kind)) {
          // Automatic links write Bazarr's configuration and reload NZBGet.
          // Finish setup before issuing tickets or snapshotting the original.
          await expect
            .poll(
              async () =>
                (await f.api('/admin/service-connections')).items
                  .filter(
                    (link) =>
                      (link.source_id === attached.id ||
                        link.target_id === attached.id) &&
                      link.enabled &&
                      link.state !== 'connected',
                  )
                  .map(
                    (link) =>
                      `${link.source_kind} -> ${link.target_kind}: ${link.state}`,
                  ),
              { timeout: 180000, intervals: [1000] },
            )
            .toEqual([]);
        }
        const group = ['radarr', 'sonarr', 'lidarr'].includes(kind)
          ? 'managers'
          : 'support';
        const record = async () =>
          (await f.api(`/admin/${group}`)).items.find(
            (s) => s.id === attached.id,
          );
        const native = kind === 'nzbget' || oldBase !== '';
        const mount = kind === 'nzbget' ? '/services/nzbget' : oldBase;
        expect((await record()).access_url).toBe(
          native ? `/services/${kind}` : null,
        );
        expect((await record()).url_base).toBe(oldBase);
        const check = async () => {
          if (group === 'managers')
            return f.api(`/admin/managers/${attached.id}/test`, 'POST');
          await f.api(`/admin/support/${attached.id}`);
          return { healthy: true };
        };
        expect((await check()).healthy).toBe(true);
        expect(
          (
            await f.context.request.get(`${f.base}/services/${kind}`, {
              maxRedirects: 0,
            })
          ).status(),
        ).toBe(native ? 307 : 404);
        const device = await f.browser.newContext({ ignoreHTTPSErrors: true });
        const browser = await f.browser.newContext({ ignoreHTTPSErrors: true });
        try {
          const login = await f.api(
            '/auth/login',
            'POST',
            {
              username: 'admin',
              password: 'test-only long passphrase',
              transport: 'device',
              device_name: 'Ownership test',
            },
            device.request,
          );
          const ticketResponse = await device.request.post(
            `${f.base}/api/v1/admin/managers/${attached.id}/access-ticket`,
            {
              headers: {
                'X-Thelxinoe-Client': '1',
                Authorization: `Bearer ${login.token}`,
              },
            },
          );
          expect(ticketResponse.status()).toBe(native ? 200 : 404);
          if (native) {
            const ticket = await ticketResponse.json();
            const tab = await browser.newPage();
            const exchange = tab.waitForResponse(
              (response) =>
                response.url() === `${f.base}${ticket.path}` &&
                response.request().method() === 'POST',
            );
            await tab.goto(`${f.base}${ticket.path}#${ticket.ticket}`);
            const exchanged = await exchange;
            expect(
              exchanged.status(),
              exchanged.ok() ? undefined : await exchanged.text(),
            ).toBe(200);
            await expect(
              kind === 'nzbget'
                ? tab.locator('#ConfigTabLink')
                : tab.getByRole('link', { name: 'Settings', exact: true }),
            ).toBeVisible({ timeout: 30000 });
            expect(new URL(tab.url()).pathname.startsWith(`${mount}/`)).toBe(
              true,
            );
            await tab.reload();
            await expect(
              kind === 'nzbget'
                ? tab.locator('#ConfigTabLink')
                : tab.getByRole('link', { name: 'Settings', exact: true }),
            ).toBeVisible({ timeout: 30000 });
            expect(
              (await browser.request.get(`${f.base}/api/v1/auth/me`)).status(),
            ).toBe(401);
            expect(
              (await browser.cookies()).some(
                (c) => c.path === `${mount}/` && c.httpOnly && c.secure,
              ),
            ).toBe(true);
            await tab.screenshot({
              path: `${output}/attached-${kind}.png`,
              fullPage: true,
              mask: [tab.locator('input')],
            });
            await device.request.post(`${f.base}/api/v1/auth/logout`, {
              headers: {
                'X-Thelxinoe-Client': '1',
                Authorization: `Bearer ${login.token}`,
              },
            });
            expect(
              (await browser.request.get(`${f.base}${mount}/`)).status(),
            ).toBe(401);
          }
        } finally {
          await browser.close();
          await device.close();
        }
        expect(
          (
            await f.context.request.put(
              `${f.base}/api/v1/admin/${group}/${attached.id}/access`,
              {
                headers: { 'X-Thelxinoe-Client': '1' },
                data: { url_base: `/services/${kind}` },
              },
            )
          ).status(),
        ).toBe(404);
        await page.goto(`${f.base}/?section=Settings`);
        await page
          .getByRole('button', { name: 'Media services', exact: true })
          .click();
        await page
          .getByRole('navigation', { name: 'Select service' })
          .getByRole('button', { name: new RegExp(`^${kind}$`, 'i') })
          .click();
        await expect(
          page.getByText('Service access', { exact: true }),
        ).toHaveCount(0);
        await expect(
          page.getByRole('link', { name: new RegExp(`^Open ${kind}`, 'i') }),
        ).toHaveCount(native ? 1 : 0);
        if (kind === 'radarr') {
          await page
            .getByRole('button', {
              name: 'Review ownership transfer',
              exact: true,
            })
            .click();
          await expect(
            page.getByRole('region', { name: 'Ownership review' }),
          ).toContainText('/services/radarr');
          await page.screenshot({
            path: `${output}/ownership-review.png`,
            fullPage: true,
          });
        }
        const filename =
          kind === 'bazarr'
            ? 'config/config.yaml'
            : kind === 'nzbget'
              ? 'nzbget.conf'
              : 'config.xml';
        const original = readFileSync(
          `${attached.directory}/${filename}`,
          'utf8',
        );
        const tag =
          group === 'managers' || kind === 'prowlarr'
            ? await attached.direct('tag', 'POST', {
                label: 'preserved-on-takeover',
              })
            : null;
        if (group === 'managers') {
          await expect
            .poll(
              async () =>
                (await attached.direct('command')).filter((c) =>
                  ['queued', 'started'].includes(c.status),
                ).length,
              { timeout: 90000 },
            )
            .toBe(0);
        }
        if (kind === 'radarr') {
          for (const source of (await f.api('/admin/support')).items.filter(
            (s) => ['prowlarr', 'bazarr'].includes(s.kind),
          )) {
            await f.api('/admin/service-connections', 'POST', {
              source_id: source.id,
              target_id: attached.id,
              action: 'connect',
            });
          }
          await expect
            .poll(
              async () =>
                (await f.api('/admin/service-connections')).items
                  .filter((link) => link.target_id === attached.id)
                  .map(
                    (link) =>
                      `${link.source_kind}: ${link.state}${link.error ? ` (${link.error})` : ''}`,
                  )
                  .sort(),
              { timeout: 90000 },
            )
            .toEqual(['bazarr: connected', 'prowlarr: connected']);
        }
        if (kind === 'sonarr') {
          await scenario(
            'failed adoption after API registration restores the original prefix',
            async () => {
              const sql = (statement) =>
                f.compose(
                  'exec',
                  '-T',
                  'server',
                  'python',
                  '-c',
                  "import sqlite3,sys; c=sqlite3.connect('/var/lib/thelxinoe/thelxinoe.sqlite3'); c.execute(sys.argv[1]); c.commit()",
                  statement,
                );
              const failed = await f.api('/admin/stack/adopt/preview', 'POST', {
                service_id: attached.id,
              });
              // Fail the ownership acceptance write after the replacement's API
              // connection has been registered, leaving a real recoverable transfer.
              sql(
                "CREATE TRIGGER fail_adoption_acceptance BEFORE UPDATE OF service_id ON stack_provisions WHEN NEW.kind='sonarr' BEGIN SELECT RAISE(ABORT, 'fixture acceptance failure'); END",
              );
              try {
                await f.api('/admin/stack/adopt', 'POST', {
                  service_id: attached.id,
                  review_id: failed.review_id,
                });
                await expect
                  .poll(
                    async () =>
                      (await f.stack()).provisions.find(
                        (p) => p.id === failed.review_id,
                      )?.state,
                    { timeout: 180000, intervals: [2000] },
                  )
                  .toBe('blocked');
                expect((await record()).url_base).toBe('/services/sonarr');
                expect((await record()).access_url).toBeNull();
              } finally {
                sql('DROP TRIGGER fail_adoption_acceptance');
              }
              await expect
                .poll(
                  async () => {
                    try {
                      await f.api(
                        `/admin/stack/${failed.review_id}/restore-original`,
                        'POST',
                        {},
                      );
                      return true;
                    } catch {
                      return false;
                    }
                  },
                  { timeout: 30000 },
                )
                .toBe(true);
              expect((await record()).url_base).toBe(oldBase);
              expect((await record()).container_id).toBe(attached.container);
              expect((await record()).access_url).toBe('/services/sonarr');
              await expect
                .poll(
                  async () => {
                    try {
                      return (await check()).healthy;
                    } catch {
                      return false;
                    }
                  },
                  { timeout: 90000 },
                )
                .toBe(true);
            },
          );
        }
        const review = await f.api('/admin/stack/adopt/preview', 'POST', {
          service_id: attached.id,
        });
        await f.api('/admin/stack/adopt', 'POST', {
          service_id: attached.id,
          review_id: review.review_id,
        });
        let stack;
        await expect
          .poll(
            async () => {
              stack = await f.stack();
              const provision = stack.provisions.find(
                (p) => p.id === review.review_id,
              );
              if (provision?.state === 'blocked')
                throw Error(`${kind}: ${provision.error}`);
              return provision?.state;
            },
            { timeout: 180000, intervals: [2000] },
          )
          .toBe('complete');
        f.services[kind] = {
          ...stack.items.find((s) => s.id === review.review_id),
          host_port: attached.port,
        };
        expect(readFileSync(`${attached.directory}/${filename}`, 'utf8')).toBe(
          original,
        );
        expect(
          JSON.parse(docker('inspect', attached.container))[0].State.Running,
        ).toBe(false);
        expect((await record()).url_base).toBe(
          kind === 'nzbget' ? '' : `/services/${kind}`,
        );
        expect((await record()).access_url).toBe(`/services/${kind}`);
        expect((await check()).healthy).toBe(true);
        if (kind === 'radarr') {
          await expect
            .poll(
              async () => {
                const applications = await f.upstream(
                  'prowlarr',
                  'applications',
                );
                const fields =
                  applications.find((app) => app.implementation === 'Radarr')
                    ?.fields ?? [];
                return fields
                  .find((field) => field.name === 'baseUrl')
                  ?.value?.endsWith('/services/radarr');
              },
              { timeout: 90000 },
            )
            .toBe(true);
          await expect
            .poll(
              async () =>
                (
                  await f.upstream('bazarr', 'system/settings')
                ).radarr.base_url.replace(/\/$/, ''),
              { timeout: 90000 },
            )
            .toBe('/services/radarr');
        }
        if (tag)
          expect(
            (await f.upstream(kind, 'tag')).some(
              (item) => item.id === tag.id && item.label === tag.label,
            ),
          ).toBe(true);
        await page.goto(`${f.base}/services/${kind}`);
        await expect(
          kind === 'nzbget'
            ? page.locator('#ConfigTabLink')
            : page.getByRole('link', { name: 'Settings', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        expect(
          new URL(page.url()).pathname.startsWith(`/services/${kind}/`),
        ).toBe(true);
        await page.reload();
        await expect(
          kind === 'nzbget'
            ? page.locator('#ConfigTabLink')
            : page.getByRole('link', { name: 'Settings', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        await page.screenshot({
          path: `${output}/adopted-${kind}.png`,
          fullPage: true,
          mask: [page.locator('input')],
        });
      },
    );
  }
}

export async function attachedConflicts({ f, page, scenario, output }) {
  await scenario(
    'attached prefixes preserve API access and never shadow application or service routes',
    async () => {
      await retireService(f, 'sonarr');
      const radarr = await f.attach('radarr', '/api', f.services.radarr.image);
      const sonarr = await f.attach(
        'sonarr',
        '/shared/nested',
        f.services.sonarr.image,
      );
      const record = async (kind) =>
        (await f.api('/admin/managers')).items.find((s) => s.kind === kind);
      const healthy = async () => {
        for (const service of [radarr, sonarr])
          expect(
            (await f.api(`/admin/managers/${service.id}/test`, 'POST')).healthy,
          ).toBe(true);
      };
      const unavailable = async (kind) => {
        expect((await record(kind)).access_url).toBeNull();
        expect(
          (
            await f.context.request.get(`${f.base}/services/${kind}`, {
              maxRedirects: 0,
            })
          ).status(),
        ).toBe(404);
      };
      await healthy();
      await unavailable('radarr');
      expect((await record('sonarr')).access_url).toBe('/services/sonarr');
      // The public API still belongs to Thelxinoe with an attached /api service.
      expect((await f.api('/auth/me')).user.role).toBe('admin');
      await radarr.setBase('/shared');
      await healthy();
      await unavailable('radarr');
      await unavailable('sonarr');
      // Also cover two services with exactly the same private prefix.
      await sonarr.setBase('/shared');
      await healthy();
      await unavailable('radarr');
      await unavailable('sonarr');
      await sonarr.setBase('/separate-sonarr');
      await healthy();
      for (const [kind, prefix] of [
        ['radarr', '/shared'],
        ['sonarr', '/separate-sonarr'],
      ]) {
        expect((await record(kind)).access_url).toBe(`/services/${kind}`);
        await page.goto(`${f.base}/services/${kind}`);
        await expect(
          page.getByRole('link', { name: 'Settings', exact: true }),
        ).toBeVisible({ timeout: 30000 });
        expect(new URL(page.url()).pathname.startsWith(`${prefix}/`)).toBe(
          true,
        );
      }
      await page.screenshot({
        path: `${output}/attached-conflicts-resolved.png`,
        fullPage: true,
        mask: [page.locator('input')],
      });
    },
  );
}
