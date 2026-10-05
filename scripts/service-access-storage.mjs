import { expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
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
    const review = await f.api('/admin/stack/adopt/preview', 'POST', {
      service_id: attached.id,
    });
    const adopted = await f.api('/admin/stack/adopt', 'POST', {
      service_id: attached.id,
      review_id: review.review_id,
    });
    await expect
      .poll(
        async () =>
          (await f.stack()).provisions.find((row) => row.id === adopted.id)
            ?.state,
        { timeout: 90000 },
      )
      .toBe('complete');
    const after = JSON.parse(docker('inspect', attached.container))[0];
    expect(after.Id).toBe(original.Id);
    expect(after.Config).toEqual(original.Config);
    expect(after.HostConfig).toEqual(original.HostConfig);
    expect(after.State.StartedAt).toBe(original.State.StartedAt);
    expect(
      (await attached.direct('rootfolder')).some(
        (root) => root.path === '/movies',
      ),
    ).toBe(true);
    await f.api(`/admin/stack/${adopted.id}/action`, 'POST', {
      action: 'release',
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
    execFileSync(
      'ffmpeg',
      [
        '-hide_banner',
        '-loglevel',
        'error',
        '-y',
        '-f',
        'lavfi',
        '-i',
        'color=c=blue:s=160x90:d=1',
        '-c:v',
        'mpeg4',
        `${f.root}/existing-movies/Mapping fixture (2000).mp4`,
      ],
      { stdio: 'pipe', windowsHide: true },
    );
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
