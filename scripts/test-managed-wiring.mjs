import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { expect } from '@playwright/test';
const snapshot = JSON.parse(
  readFileSync('.local/managed-install-result.json', 'utf8'),
);
const results = [];
for (const service of snapshot.items) {
  const configPath =
    service.kind === 'bazarr'
      ? 'config/config.yaml'
      : service.kind === 'nzbget'
        ? 'nzbget.conf'
        : 'config.xml';
  const raw = execFileSync(
    'docker',
    [
      'compose',
      '-f',
      'compose.managed.test.yaml',
      'exec',
      '-T',
      '-u',
      '10001:10001',
      'controller',
      'cat',
      `/var/lib/thelxinoe/deployment/services/${service.id}/appdata/${configPath}`,
    ],
    { encoding: 'utf8' },
  );
  const port = snapshot.provisions.find((p) => p.id === service.id).host_port;
  if (['radarr', 'sonarr', 'lidarr', 'prowlarr'].includes(service.kind)) {
    const key = raw.match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
    const version = ['lidarr', 'prowlarr'].includes(service.kind) ? 1 : 3;
    const path =
      service.kind === 'prowlarr' ? 'applications' : 'downloadclient';
    const data = await (
      await fetch(`http://localhost:${port}/api/v${version}/${path}`, {
        headers: { 'X-Api-Key': key },
      })
    ).json();
    const names = data.map((s) => s.name);
    expect(names.length).toBeGreaterThan(0);
    results.push({ kind: service.kind, connections: names });
  }
  if (service.kind === 'bazarr') {
    const key = raw.match(/^\s+apikey:\s*['"]?([a-zA-Z0-9]+)/m)[1];
    const data = await (
      await fetch(`http://localhost:${port}/api/system/settings`, {
        headers: { 'X-API-KEY': key },
      })
    ).json();
    expect(data.general.use_radarr).toBe(true);
    expect(data.general.use_sonarr).toBe(true);
    results.push({
      kind: 'bazarr',
      radarr: data.radarr.ip,
      sonarr: data.sonarr.ip,
    });
  }
}
writeFileSync(
  '.local/managed-wiring-result.json',
  JSON.stringify(results, null, 2),
);
console.log(JSON.stringify(results));
