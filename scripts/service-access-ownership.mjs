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
      `${kind}: adoption preserves data and configures protected native access`,
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
        const gatewayAuth = ['radarr', 'sonarr', 'lidarr', 'prowlarr'].includes(
          kind,
        );
        const attached = await f.attach(kind, oldBase, image, {
          authentication: gatewayAuth ? 'Forms' : 'External',
          nzbgetWarnings: kind === 'nzbget',
        });
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
                      `${link.source_kind} -> ${link.target_kind}: ${link.state}${link.error ? ` (${link.error})` : ''}`,
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
              gatewayAuth
                ? tab.locator('input[type="password"]')
                : kind === 'nzbget'
                  ? tab.locator('#ConfigTabLink')
                  : tab.getByRole('link', { name: 'Settings', exact: true }),
            ).toBeVisible({ timeout: 30000 });
            expect(new URL(tab.url()).pathname.startsWith(`${mount}/`)).toBe(
              true,
            );
            await tab.reload();
            await expect(
              gatewayAuth
                ? tab.locator('input[type="password"]')
                : kind === 'nzbget'
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
          .getByRole('link', { name: 'Media services', exact: true })
          .click();
        await page
          .getByRole('navigation', { name: 'Select service' })
          .getByRole('link', { name: new RegExp(`^${kind}$`, 'i') })
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
          await expect(
            page.getByRole('region', { name: 'Ownership review' }),
          ).toContainText('Thelxinoe login');
          await expect(
            page.getByRole('region', { name: 'Ownership review' }),
          ).toContainText('Published ports are removed');
          await page.screenshot({
            path: `${output}/ownership-review.png`,
            fullPage: true,
          });
        }
        if (kind === 'nzbget') {
          await scenario(
            'NZBGet rejects an invalid certificate store without blocking adoption review',
            async () => {
              const config = await attached.direct('loadconfig');
              const store = config.find((row) => row.Name === 'CertStore');
              const saved = store.Value;
              store.Value = '/config/invalid-ca.pem';
              await attached.direct('saveconfig', 'POST', [config]);
              await attached.direct('reload');
              await expect
                .poll(
                  async () => {
                    try {
                      return (await attached.direct('config')).find(
                        (row) => row.Name === 'CertStore',
                      )?.Value;
                    } catch {
                      return null;
                    }
                  },
                  { timeout: 30000 },
                )
                .toBe('/config/invalid-ca.pem');
              const invalid = await f.api(
                '/admin/stack/adopt/preview',
                'POST',
                { service_id: attached.id },
              );
              expect(invalid.nzbget.cert_store).toBeNull();
              await expect(
                f.api('/admin/stack/adopt', 'POST', {
                  service_id: attached.id,
                  review_id: invalid.review_id,
                  nzbget: { rotate_logs: true, cert_check: true },
                }),
              ).rejects.toThrow();
              expect(
                JSON.parse(docker('inspect', attached.container))[0].State
                  .Running,
              ).toBe(true);
              store.Value = saved;
              await attached.direct('saveconfig', 'POST', [config]);
              await attached.direct('reload');
              await expect
                .poll(
                  async () => {
                    try {
                      return (await attached.direct('config')).find(
                        (row) => row.Name === 'CertStore',
                      )?.Value;
                    } catch {
                      return null;
                    }
                  },
                  { timeout: 30000 },
                )
                .toBe(saved);
            },
          );
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
              expect(
                readFileSync(`${attached.directory}/${filename}`, 'utf8'),
              ).toBe(original);
              expect(
                JSON.parse(docker('inspect', attached.container))[0]
                  .NetworkSettings.Ports['8989/tcp'][0].HostPort,
              ).toBe(String(attached.port));
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
        if (kind === 'nzbget') {
          await scenario(
            'NZBGet can skip warning fixes and recover its original credentials after failed adoption',
            async () => {
              const skipped = await f.api(
                '/admin/stack/adopt/preview',
                'POST',
                {
                  service_id: attached.id,
                },
              );
              expect(skipped.nzbget.append_log).toBe(true);
              expect(skipped.nzbget.empty_password).toBe(true);
              expect(skipped.nzbget.cert_check_disabled).toBe(true);
              expect(skipped.nzbget.cert_store).toMatch(/^\//);
              const sql = (statement) =>
                f.compose(
                  'exec',
                  '-T',
                  'server',
                  'python',
                  '-c',
                  'import sqlite3,sys; db=sqlite3.connect("/var/lib/thelxinoe/thelxinoe.sqlite3"); db.execute(sys.argv[1]); db.commit()',
                  statement,
                );
              sql(
                "CREATE TRIGGER fail_nzbget_adoption BEFORE UPDATE OF service_id ON stack_provisions WHEN NEW.kind='nzbget' BEGIN SELECT RAISE(ABORT, 'fixture acceptance failure'); END",
              );
              try {
                await f.api('/admin/stack/adopt', 'POST', {
                  service_id: attached.id,
                  review_id: skipped.review_id,
                  nzbget: { rotate_logs: false, cert_check: false },
                });
                let failed;
                await expect
                  .poll(
                    async () => {
                      failed = await f.stack();
                      return failed.provisions.find(
                        (p) => p.id === skipped.review_id,
                      )?.state;
                    },
                    { timeout: 180000 },
                  )
                  .toBe('blocked');
                f.services.nzbget = {
                  ...failed.items.find((s) => s.id === skipped.review_id),
                  host_port: attached.port,
                };
                const copied = f.config('nzbget');
                expect(copied).toMatch(/^WriteLog=append$/m);
                expect(copied).toMatch(/^CertCheck=no$/m);
                expect(copied).toMatch(/^ControlUsername=thelxinoe$/m);
                await f.api(
                  `/admin/stack/${skipped.review_id}/restore-original`,
                  'POST',
                );
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
                const login = await f.api(
                  `/admin/support/${attached.id}/login`,
                  'POST',
                );
                expect(login).toEqual({ username: 'fixture', password: '' });
                expect(
                  readFileSync(`${attached.directory}/${filename}`, 'utf8'),
                ).toBe(original);
              } finally {
                sql('DROP TRIGGER fail_nzbget_adoption');
              }
            },
          );
        }
        const review = await f.api('/admin/stack/adopt/preview', 'POST', {
          service_id: attached.id,
        });
        expect(review.authentication).toBe(
          gatewayAuth
            ? 'external'
            : kind === 'nzbget'
              ? 'generated'
              : 'preserved',
        );
        expect(review.publish_ports).toBe(!gatewayAuth);
        await f.api('/admin/stack/adopt', 'POST', {
          service_id: attached.id,
          review_id: review.review_id,
          ...(kind === 'nzbget'
            ? { nzbget: { rotate_logs: true, cert_check: true } }
            : {}),
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
          host_port: gatewayAuth ? null : attached.port,
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
        if (gatewayAuth) {
          const managed = JSON.parse(
            docker('inspect', f.services[kind].container_id),
          )[0];
          expect(
            Object.values(managed.NetworkSettings.Ports ?? {})
              .flat()
              .filter(Boolean),
          ).toEqual([]);
          const host = await f.upstream(kind, 'config/host');
          expect(host.authenticationMethod.toLowerCase()).toBe('external');
          expect(host.authenticationRequired.toLowerCase()).toBe('enabled');
          if (typeof host.allowedHosts === 'string') {
            const allowed = host.allowedHosts
              .split(/[;,]/)
              .map((value) => value.trim());
            expect(allowed).toContain('custom.example');
            expect(allowed).toContain(managed.Name.slice(1));
            expect(allowed).toContain(`thelxinoe-${kind}`);
            expect(allowed).toContain(
              JSON.parse(docker('inspect', attached.container))[0].Name.slice(
                1,
              ),
            );
            expect(review.allowed_hosts).toBe(true);
            expect(
              f.compose(
                'exec',
                '-T',
                'server',
                'curl',
                '-s',
                '-o',
                '/dev/null',
                '-w',
                '%{http_code}',
                '-H',
                'Host: unreviewed.example',
                `http://${managed.Name.slice(1)}:${(await record()).port}/services/${kind}/`,
              ),
            ).toBe('400');
          } else {
            expect(review.allowed_hosts).toBe(false);
          }
          await expect(
            f.context.request.get(
              `http://localhost:${attached.port}${oldBase}/`,
              { timeout: 3000 },
            ),
          ).rejects.toThrow();
          const anonymous = await f.browser.newContext({
            ignoreHTTPSErrors: true,
          });
          try {
            expect(
              (
                await anonymous.request.get(`${f.base}/services/${kind}/`)
              ).status(),
            ).toBe(401);
          } finally {
            await anonymous.close();
          }
        } else if (kind === 'nzbget') {
          const config = f.config(kind);
          const password = config.match(/^ControlPassword=(.*)$/m)[1];
          expect(config).toMatch(/^ControlUsername=thelxinoe$/m);
          expect(password).toMatch(/^[a-f0-9]{32}$/);
          expect(config).toMatch(/^WriteLog=rotate$/m);
          expect(config).toMatch(/^RotateLog=3$/m);
          expect(config).toMatch(/^CertCheck=yes$/m);
          expect(config).toContain(`CertStore=${review.nzbget.cert_store}`);
          expect(config).toContain('DestDir=${MainDir}/completed');
          expect(config).toContain('Category7.DestDir=${MainDir}/custom');
          expect(
            await f.api(`/admin/stack/${review.review_id}/login`, 'POST'),
          ).toEqual({ username: 'thelxinoe', password });
          expect(
            (await f.upstream('nzbget', 'config')).find(
              (row) => row.Name === 'CertCheck',
            ).Value,
          ).toBe('yes');
          expect(
            (
              await f.context.request.post(
                `http://localhost:${attached.port}/jsonrpc`,
                { data: { method: 'version', params: [], id: 1 } },
              )
            ).status(),
          ).toBe(401);
          await expect
            .poll(
              async () =>
                (await f.api('/admin/service-connections')).items
                  .filter(
                    (link) => link.target_id === attached.id && link.enabled,
                  )
                  .map((link) => link.state),
              { timeout: 180000 },
            )
            .toEqual(['connected', 'connected', 'connected']);
          for (const source of ['radarr', 'sonarr', 'lidarr']) {
            await expect
              .poll(
                async () => {
                  const clients = await f.upstream(source, 'downloadclient');
                  return clients
                    .find((row) => row.implementation === 'Nzbget')
                    ?.fields.find((field) => field.name === 'username')?.value;
                },
                { timeout: 90000 },
              )
              .toBe('thelxinoe');
            const clients = await f.upstream(source, 'downloadclient');
            const client = clients.find(
              (row) => row.implementation === 'Nzbget',
            );
            await f.upstream(source, 'downloadclient/test', 'POST', client);
          }
        }
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
