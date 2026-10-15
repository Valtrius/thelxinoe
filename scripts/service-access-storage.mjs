import { expect } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { docker, fixture, freePort } from './service-access-fixture.mjs';
import { imageManifest } from './ci-images.mjs';
import { waitForServerTools } from './ci-readiness.mjs';

export async function existingStorageAccess(
  output = 'test-results/service-access',
) {
  const f = await fixture({ separateMovies: true });
  const result = { passed: false, project: f.project };
  try {
    const attached = await f.attach(
      'radarr',
      '/existing-radarr',
      imageManifest.services.radarr,
      { existingLayout: true },
    );
    await attached.direct('rootfolder', 'POST', { path: '/movies' });
    const original = JSON.parse(docker('inspect', attached.container))[0];
    // Existing /movies storage is kept in place by the Owned replacement.
    const { view, review } = await f.transfer(attached.id);
    expect(view.state, view.error ?? '').toBe('complete');
    expect(
      review.review.plan.target.mounts.map((mount) => mount.destination),
    ).toContain('/movies');
    const retained = JSON.parse(docker('inspect', attached.container))[0];
    expect(retained.State.Running).toBe(false);
    const runtime = (await f.stack()).items.find(
      (item) => item.kind === 'radarr',
    );
    const replacement = JSON.parse(docker('inspect', runtime.container_id))[0];
    expect(
      replacement.Mounts.find((mount) => mount.Destination === '/movies')
        ?.Source,
    ).toBe(
      original.Mounts.find((mount) => mount.Destination === '/movies').Source,
    );
    const roots = JSON.parse(
      docker(
        'exec',
        runtime.container_id,
        'curl',
        '-fsS',
        '-H',
        `X-Api-Key: ${attached.key}`,
        'http://localhost:7878/services/radarr/api/v3/rootfolder',
      ),
    );
    expect(roots.some((root) => root.path === '/movies')).toBe(true);
    // Retire it so the media mapping below can use the API fixture peer.
    const provision = (await f.api('/admin/services')).items.find(
      (item) => item.kind === 'radarr',
    ).ownership.id;
    await f.api(`/admin/stack/${provision}/action`, 'POST', { action: 'stop' });
    docker('rm', runtime.container_id);
    await f.api(`/admin/stack/${provision}/action`, 'POST', {
      action: 'retire',
    });
    const blocked = f.compose(
      'exec',
      '-T',
      'controller',
      'curl',
      '-sS',
      '--unix-socket',
      '/run/thelxinoe/controller.sock',
      '-H',
      'Content-Type: application/json',
      '-w',
      '\n%{http_code}',
      '--data-binary',
      JSON.stringify({
        kind: 'radarr',
        operation_id: randomUUID(),
        host_port: await freePort(),
      }),
      'http://localhost/stack/install',
    );
    expect(blocked.endsWith('\n409')).toBe(true);
    expect(blocked).toContain('Fresh installations require');
    f.createMappingMovie();
    await f.peer({ existingLayout: true });
    await f.api('/catalog/roots', 'POST', {
      name: 'Existing movies',
      kind: 'movies',
      path: '/media/movies',
    });
    const root = (await f.api('/catalog/roots')).items.find(
      (row) => row.path === '/media/movies',
    );
    await waitForServerTools((timeout) =>
      f.api('/admin/tools', 'GET', undefined, f.context.request, timeout),
    );
    const scan = await f.api(`/catalog/roots/${root.id}/scan`, 'POST');
    await expect
      .poll(
        async () =>
          (await f.api('/admin/jobs')).items.find(
            (row) => row.id === scan.job_id,
          )?.state,
        { timeout: 90000 },
      )
      .toBe('complete');
    await f.api('/admin/managers/reconcile', 'POST');
    const file = (await f.api('/admin/managers/bindings')).items.find(
      (row) => row.path === '/media/movies/Mapping fixture (2000).mp4',
    );
    expect(file.ownership).toBe('managed');
    expect(file.bindings.some((binding) => binding.file_id === 11)).toBe(true);
    result.passed = true;
    result.container_id = original.Id;
    result.source = '/movies/Mapping fixture (2000).mp4';
    result.server = file.path;
    result.ownership = file.ownership;
    result.fresh_installation = 'blocked before creating a container';
    result.transfer = {
      checks: view.checks,
      mounts: review.review.plan.target.mounts,
    };
  } catch (error) {
    result.error = error.message;
    result.jobs = await f.api('/admin/jobs').catch(() => null);
    result.tools = await f.api('/admin/tools').catch(() => null);
    throw error;
  } finally {
    mkdirSync(output, { recursive: true });
    writeFileSync(
      `${output}/existing-storage.json`,
      JSON.stringify(result, null, 2),
    );
    await f.close();
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
)
  await existingStorageAccess();
