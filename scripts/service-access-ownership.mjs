import { expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { docker } from './service-access-fixture.mjs';

const internalPort = {
  radarr: 7878,
  sonarr: 8989,
  lidarr: 8686,
  prowlarr: 9696,
  bazarr: 6767,
  nzbget: 6789,
};

export async function retireService(f, kind) {
  if (
    ['radarr', 'sonarr', 'lidarr'].includes(kind) &&
    JSON.parse(docker('inspect', f.services[kind].container_id))[0].State
      .Running
  ) {
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

async function stage(f, kind) {
  return (await f.api('/admin/services')).items.find(
    (item) => item.kind === kind,
  );
}

/** Point the shared fixture at the Owned replacement after a transfer. */
async function adoptReplacement(f, kind, image) {
  const owned = await stage(f, kind);
  const runtime = (await f.stack()).items.find((item) => item.kind === kind);
  const raw = JSON.parse(docker('inspect', runtime.container_id))[0];
  const published =
    raw.NetworkSettings.Ports?.[`${internalPort[kind]}/tcp`]?.[0]?.HostPort;
  f.services[kind] = {
    ...runtime,
    id: owned.ownership.id,
    image,
    container_id: runtime.container_id,
    host_port: published ? Number(published) : null,
  };
  return f.services[kind];
}

// test-transfers.mjs transfers every kind; these cover the kind-specific
// review blockers and the Connected stage before a transfer.
export async function ownershipAccess({ f, scenario, output }) {
  for (const kind of ['radarr', 'sonarr', 'nzbget']) {
    await scenario(
      kind +
        ': a Connected service is validation-only until its transfer completes',
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
        const configPath =
          attached.directory +
          (kind === 'bazarr'
            ? '/config/config.yaml'
            : kind === 'nzbget'
              ? '/nzbget.conf'
              : '/config.xml');
        const config = () => readFileSync(configPath, 'utf8');
        const root =
          kind === 'radarr' ? '/movies' : kind === 'sonarr' ? '/tv' : null;
        if (root) await attached.direct('rootfolder', 'POST', { path: root });
        const original = inspect();
        const group = ['radarr', 'sonarr', 'lidarr'].includes(kind)
          ? 'managers'
          : 'support';
        const integration = async () =>
          (await f.api('/admin/' + group)).items.find(
            (s) => s.id === attached.id,
          );

        // Connected: health and identity only, with no access or operations.
        expect((await stage(f, kind)).stage).toBe('connected');
        expect(
          (await f.api(`/admin/services/${attached.id}/test`, 'POST')).healthy,
        ).toBe(true);
        expect(await integration()).toMatchObject({
          operational: false,
          access_url: null,
        });
        const links = (await f.api('/admin/service-connections')).items.filter(
          (l) => l.source_id === attached.id || l.target_id === attached.id,
        );
        expect(links.every((l) => !l.enabled)).toBe(true);
        const after = inspect();
        expect(after.Config).toEqual(original.Config);
        expect(after.HostConfig).toEqual(original.HostConfig);
        expect(after.State.StartedAt).toBe(original.State.StartedAt);

        // A stopped original can't be reviewed; nothing changes until it's back.
        if (kind === 'sonarr') {
          docker('stop', attached.container);
          const blocked = await f.transfer(attached.id, { approve: false });
          expect(blocked.review.review.blockers.join(' ')).toContain(
            'Start the original',
          );
          await f.api(`/admin/transfers/${blocked.review.id}/cancel`, 'POST');
          docker('start', attached.container);
          // Errors are expected until the restarted original answers again.
          await expect
            .poll(
              async () =>
                (
                  await f
                    .api(`/admin/services/${attached.id}/test`, 'POST')
                    .catch((error) => ({ healthy: error.message }))
                ).healthy,
              { timeout: 180000 },
            )
            .toBe(true);
        }
        // Compose ownership must be explicitly resolved before anything stops.
        if (kind === 'radarr') {
          const { review } = await f.transfer(attached.id, { approve: false });
          expect(review.review.plan.compose).toBeTruthy();
          await expect(
            f.api(`/admin/transfers/${review.id}/approve`, 'POST', {
              review_revision: review.review_revision,
              confirm: true,
              declarations: {
                cron: true,
                systemd: true,
                ci_cd: true,
                configuration_management: true,
                manual_compose: true,
                unobservable_automation: true,
              },
            }),
          ).rejects.toThrow(/HTTP 4\d\d/);
          expect(inspect().State.StartedAt).toBe(original.State.StartedAt);
        }

        const before = config();
        const { view, review } = await f.transfer(attached.id);
        expect(view.state, view.error ?? '').toBe('complete');
        // The original is kept stopped and unchanged, with restart disabled.
        const retained = inspect();
        expect(retained.State.Running).toBe(false);
        expect(retained.HostConfig.RestartPolicy.Name).toBe('no');
        expect(config()).toBe(before);
        const owned = await integration();
        expect(owned.operational).toBe(true);
        expect(owned.container_id).not.toBe(original.Id);
        if (kind !== 'nzbget') expect(owned.url_base).toBe(`/services/${kind}`);
        const replacement = await adoptReplacement(f, kind, image);
        if (root)
          expect(
            (await f.upstream(kind, 'rootfolder')).map((r) => r.path),
          ).toContain(root);
        if (kind === 'nzbget') {
          const login = await f.api(
            `/admin/stack/${replacement.id}/login`,
            'POST',
          );
          expect(login.username).toBe('fixture');
          expect(login.password).toBe('');
        }
        writeFileSync(
          output + '/ownership-' + kind + '.json',
          JSON.stringify(
            {
              original: original.Id,
              replacement: replacement.container_id,
              original_retained_stopped: true,
              checks: view.checks,
              changes: review.review.plan.changes,
              settings: review.review.plan.settings,
            },
            null,
            2,
          ),
        );
      },
    );
  }
}

export async function attachedConflicts({ f, scenario }) {
  await scenario(
    'Connected services never get native access or service routes',
    async () => {
      await retireService(f, 'radarr');
      const radarr = await f.attach('radarr', '/api', f.services.radarr.image);
      const record = (await f.api('/admin/managers')).items.find(
        (s) => s.id === radarr.id,
      );
      expect(record.access_url).toBeNull();
      expect(
        (
          await f.context.request.get(`${f.base}/services/radarr`, {
            maxRedirects: 0,
          })
        ).status(),
      ).toBe(404);
      // Operations stay unavailable, while the public API still belongs to Thelxinoe.
      await expect(
        f.api(`/admin/managers/${radarr.id}/options`),
      ).rejects.toThrow('HTTP 409');
      expect((await f.api('/auth/me')).user.role).toBe('admin');
      await f.api(`/admin/services/${radarr.id}/forget`, 'POST');
      expect(
        (await f.api('/admin/managers')).items.some((s) => s.id === radarr.id),
      ).toBe(false);
      // Forgetting never touches the externally managed container.
      expect(
        JSON.parse(docker('inspect', radarr.container))[0].State.Running,
      ).toBe(true);
      docker('rm', '-f', radarr.container);
    },
  );
}
