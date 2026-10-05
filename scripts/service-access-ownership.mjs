import { expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { docker } from './service-access-fixture.mjs';

export async function retireService(f, kind) {
  if (
    ['radarr', 'sonarr', 'lidarr'].includes(kind) &&
    JSON.parse(docker('inspect', f.services[kind].container_id))[0].State
      .Running
  ) {
    await expect
      .poll(
        async () =>
          (
            await (f.services[kind].direct
              ? f.services[kind].direct('command')
              : f.upstream(kind, 'command'))
          ).filter((c) => ['queued', 'started'].includes(c.status)).length,
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

export async function ownershipAccess({ f, scenario, output }) {
  for (const kind of [
    'radarr',
    'sonarr',
    'lidarr',
    'prowlarr',
    'bazarr',
    'nzbget',
  ]) {
    await scenario(
      kind + ': ownership preserves the existing deployment and application',
      async () => {
        const image = f.services[kind].image;
        await retireService(f, kind);
        const prefix = kind === 'nzbget' ? '' : '/existing-' + kind;
        const attached = await f.attach(kind, prefix, image, {
          authentication: ['radarr', 'sonarr', 'lidarr', 'prowlarr'].includes(
            kind,
          )
            ? 'Forms'
            : 'External',
          nzbgetWarnings: kind === 'nzbget',
          existingLayout: kind === 'radarr' || kind === 'sonarr',
          composeOwnership: kind === 'radarr',
        });
        const inspect = () =>
          JSON.parse(docker('inspect', attached.container))[0];
        const config = () =>
          readFileSync(
            attached.directory +
              (kind === 'bazarr'
                ? '/config/config.yaml'
                : kind === 'nzbget'
                  ? '/nzbget.conf'
                  : '/config.xml'),
            'utf8',
          );
        const root =
          kind === 'radarr' ? '/movies' : kind === 'sonarr' ? '/tv' : null;
        if (root) await attached.direct('rootfolder', 'POST', { path: root });
        const originalRoots = root ? await attached.direct('rootfolder') : null;
        if (kind === 'sonarr') docker('stop', attached.container);
        const original = inspect();
        const originalConfig = config();
        const review = await f.api('/admin/stack/adopt/preview', 'POST', {
          service_id: attached.id,
        });
        expect(review.mode).toBe('in_place');
        expect(review.changes).toEqual([]);
        expect(review.restart_required).toBe(false);
        expect(review.authentication).toBe('preserved');
        expect(review.capabilities.lifecycle.available).toBe(true);
        if (kind === 'radarr') {
          expect(review.compose_project).toBeTruthy();
          await expect(
            f.api('/admin/stack/adopt', 'POST', {
              service_id: attached.id,
              review_id: review.review_id,
              released_compose: false,
            }),
          ).rejects.toThrow('HTTP 409');
          expect(inspect().State.StartedAt).toBe(original.State.StartedAt);
        }
        const submitted = await f.api('/admin/stack/adopt', 'POST', {
          service_id: attached.id,
          review_id: review.review_id,
          released_compose: true,
        });
        expect(
          (
            await f.api('/admin/stack/adopt', 'POST', {
              service_id: attached.id,
              review_id: review.review_id,
              released_compose: true,
            })
          ).id,
        ).toBe(submitted.id);
        await expect
          .poll(
            async () =>
              (await f.api('/admin/stack')).provisions.find(
                (p) => p.id === submitted.id,
              )?.state,
            { timeout: 90000 },
          )
          .toBe('complete');
        if (kind === 'nzbget') {
          f.compose('restart', 'controller');
          await expect
            .poll(
              async () =>
                (await f.stack().catch(() => ({ items: [] }))).items.find(
                  (entry) => entry.id === submitted.id,
                )?.container_id,
              { timeout: 90000 },
            )
            .toBe(original.Id);
          const login = await f.api(
            `/admin/stack/${submitted.id}/login`,
            'POST',
          );
          expect(login.username).toBe('fixture');
          expect(login.password).toBe('');
        }
        const after = inspect();
        expect(after.Id).toBe(original.Id);
        expect(after.Image).toBe(original.Image);
        expect(after.Config).toEqual(original.Config);
        expect(after.HostConfig).toEqual(original.HostConfig);
        expect(after.NetworkSettings.Networks).toEqual(
          original.NetworkSettings.Networks,
        );
        expect(after.State.StartedAt).toBe(original.State.StartedAt);
        expect(after.State.Running).toBe(original.State.Running);
        expect(after.RestartCount).toBe(original.RestartCount);
        expect(config()).toBe(originalConfig);
        if (root && kind !== 'sonarr') {
          expect(
            (await attached.direct('rootfolder')).map((r) => r.path),
          ).toEqual(originalRoots.map((r) => r.path));
        }
        const group = ['radarr', 'sonarr', 'lidarr'].includes(kind)
          ? 'managers'
          : 'support';
        const integration = (await f.api('/admin/' + group)).items.find(
          (s) => s.id === attached.id,
        );
        expect(integration.container_id).toBe(original.Id);
        expect(integration.url_base).toBe(prefix);
        if (group === 'managers') {
          expect(integration.retention_enabled).toBe(false);
          await f.api('/admin/managers/' + attached.id + '/retention', 'PUT', {
            enabled: true,
          });
          expect(
            (await f.api('/admin/managers')).items.find(
              (s) => s.id === attached.id,
            ).retention_enabled,
          ).toBe(true);
          await f.api('/admin/managers/' + attached.id + '/retention', 'PUT', {
            enabled: false,
          });
        }

        const links = (await f.api('/admin/service-connections')).items.filter(
          (l) => l.source_id === attached.id || l.target_id === attached.id,
        );
        expect(links.every((l) => !l.enabled)).toBe(true);
        const policy = (await f.api('/admin/service-updates')).policies?.find(
          (p) => p.service_id === submitted.id,
        );
        if (policy) expect(policy.policy).toBe('notify');
        await f.api('/admin/stack/' + submitted.id + '/action', 'POST', {
          action: 'release',
        });
        expect(inspect().Id).toBe(original.Id);
        expect(config()).toBe(originalConfig);
        expect(
          (await f.api('/admin/' + group)).items.some(
            (s) => s.id === attached.id,
          ),
        ).toBe(true);
        expect(
          (await f.api('/admin/stack')).provisions.some(
            (p) => p.id === submitted.id,
          ),
        ).toBe(false);
        writeFileSync(
          output + '/ownership-' + kind + '.json',
          JSON.stringify(
            {
              container_id: original.Id,
              retained: true,
              running: original.State.Running,
              review,
              released_without_deletion: true,
            },
            null,
            2,
          ),
        );
        const reacquire = await f.api('/admin/stack/adopt/preview', 'POST', {
          service_id: attached.id,
        });
        const owned = await f.api('/admin/stack/adopt', 'POST', {
          service_id: attached.id,
          review_id: reacquire.review_id,
          released_compose: true,
        });
        await expect
          .poll(
            async () =>
              (await f.api('/admin/stack')).provisions.find(
                (p) => p.id === owned.id,
              )?.state,
            { timeout: 90000 },
          )
          .toBe('complete');
        f.services[kind] = {
          ...f.services[kind],
          id: owned.id,
          container_id: original.Id,
          host_port: attached.port,
          direct: attached.direct,
        };
      },
    );
  }
}

export async function attachedConflicts({ f, page, scenario, output }) {
  await scenario(
    'attached prefixes preserve API access and never shadow application or service routes',
    async () => {
      await retireService(f, 'radarr');
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
      const nativeUrl = `http://localhost:${radarr.port}/api`;
      await f.api(`/admin/managers/${radarr.id}/native-url`, 'PUT', {
        url: nativeUrl,
      });
      expect((await record('radarr')).access_url).toBe(nativeUrl);
      await page.goto(nativeUrl);
      await expect(
        page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible({ timeout: 30000 });
      await f.api(`/admin/managers/${radarr.id}/native-url`, 'PUT', {
        url: '',
      });
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
