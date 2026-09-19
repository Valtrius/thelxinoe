import { readFileSync, writeFileSync } from 'node:fs';
const key = readFileSync(
  '.local/acquisition/bazarr/config/config.yaml',
  'utf8',
).match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
async function api(p, method = 'GET', body) {
  const r = await fetch('http://localhost:26767/api/' + p, {
    method,
    headers: { 'X-API-KEY': key },
    body,
  });
  if (!r.ok) throw Error(p + ': ' + r.status);
  return r.json().catch(() => null);
}
const form = new URLSearchParams();
for (const [kind, port] of [
  ['radarr', 7878],
  ['sonarr', 8989],
]) {
  const token = readFileSync(
    `.local/acquisition/${kind}/config.xml`,
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  for (const [k, v] of Object.entries({
    [`settings-general-use_${kind}`]: 'true',
    [`settings-${kind}-ip`]: kind,
    [`settings-${kind}-port`]: String(port),
    [`settings-${kind}-apikey`]: token,
    [`settings-${kind}-ssl`]: 'false',
  }))
    form.set(k, v);
}
form.set('settings-general-enabled_providers', 'embeddedsubtitles');
form.set('settings-general-use_embedded_subs', 'false');
form.set('languages-enabled', 'en');
form.set(
  'languages-profiles',
  JSON.stringify([
    {
      profileId: 1,
      name: 'Fixture English',
      cutoff: 1,
      items: [
        {
          id: 1,
          language: 'en',
          hi: 'False',
          forced: 'False',
          audio_exclude: 'False',
          audio_only_include: 'False',
        },
      ],
      mustContain: [],
      mustNotContain: [],
      originalFormat: false,
    },
  ]),
);
form.set('settings-general-movie_default_enabled', 'true');
form.set('settings-general-movie_default_profile', '1');
form.set('settings-general-serie_default_enabled', 'true');
form.set('settings-general-serie_default_profile', '1');
await api('system/settings', 'POST', form);
console.log(
  'Bazarr connected to isolated managers and embedded subtitle provider.',
);
for (const p of ['system/tasks', 'movies', 'movies/wanted']) {
  const d = await api(p);
  writeFileSync(
    '.local/bazarr-' + p.replaceAll('/', '-') + '.json',
    JSON.stringify(d, null, 2),
  );
  console.log(
    p +
      ': ' +
      JSON.stringify(
        p === 'system/tasks'
          ? d.data?.map((t) => ({ id: t.job_id, name: t.name }))
          : d.data?.map((m) => ({
              id: m.radarrId,
              title: m.title,
              profile: m.profileId,
              missing: m.missing_subtitles,
            })),
      ),
  );
}
