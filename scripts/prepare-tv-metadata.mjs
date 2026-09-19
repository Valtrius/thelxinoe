import { request } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
const api = await request.newContext({
  baseURL: 'https://localhost:21443',
  ignoreHTTPSErrors: true,
  extraHTTPHeaders: { 'X-Thelxinoe-Client': '1' },
});
async function call(path, method = 'GET', data) {
  const r = await api.fetch(`/api/v1${path}`, { method, data });
  if (!r.ok())
    throw new Error(`${method} ${path.split('?')[0]}: ${r.status()}`);
  return r.json();
}
try {
  await call('/auth/login', 'POST', {
    username: 'admin',
    password: 'test-only long passphrase',
  });
  await call('/admin/metadata', 'PUT', {
    tmdb_token: readFileSync('.local/tmdb-token', 'utf8').trim(),
    musicbrainz_contact: readFileSync(
      '.local/musicbrainz-contact',
      'utf8',
    ).trim(),
  });
  const movie = (await call('/catalog?kind=movie')).items.find(
    (i) => i.title === 'Direct',
  );
  if (!movie.metadata.artwork_cached) {
    await call(`/catalog/${movie.id}/overrides`, 'PUT', { title: 'Direct' });
    const job = (
      await call(`/catalog/${movie.id}/match`, 'POST', {
        provider: 'tmdb',
        external_id: '603',
      })
    ).job_id;
    let done = false;
    for (let i = 0; i < 120; i++) {
      const result = (await call('/admin/jobs')).items.find(
        (j) => j.id === job,
      );
      if (result?.state === 'complete') {
        done = true;
        break;
      }
      if (result?.state === 'failed')
        throw new Error('TV fixture artwork failed');
      await new Promise((r) => setTimeout(r, 500));
    }
    if (!done) throw new Error('TV fixture artwork timed out');
  }
  const history = (await call('/me/history')).items.filter((i) =>
    i.device.startsWith('Wholphin'),
  );
  writeFileSync(
    '.local/wholphin-playback-history.json',
    JSON.stringify(history, null, 2),
  );
  console.log(
    'Configured real artwork for the synthetic TV playback fixture; retained Wholphin playback evidence.',
  );
} finally {
  await api.dispose();
}
