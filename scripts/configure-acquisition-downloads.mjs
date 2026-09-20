import { readFileSync } from 'node:fs';
function arr(kind, port, version) {
  const key = readFileSync(
    `.local/acquisition/${kind}/config.xml`,
    'utf8',
  ).match(/<ApiKey>(.*?)<\/ApiKey>/)[1];
  return {
    key,
    async api(path, method = 'GET', body) {
      const r = await fetch(
        `http://127.0.0.1:${port}/api/v${version}/${path}`,
        {
          method,
          headers: { 'X-Api-Key': key, 'Content-Type': 'application/json' },
          body: body ? JSON.stringify(body) : undefined,
        },
      );
      const data = await r.json().catch(() => null);
      if (!r.ok) {
        console.log(
          JSON.stringify({
            service: kind,
            path,
            status: r.status,
            errors: Array.isArray(data)
              ? data.map((e) => ({
                  field: e.propertyName,
                  message: e.errorMessage,
                }))
              : undefined,
          }),
        );
        throw Error('Fixture configuration rejected');
      }
      return data;
    },
  };
}
const raw = readFileSync('.local/acquisition/nzbget/nzbget.conf', 'utf8');
const username = raw.match(/^ControlUsername=(.*)$/m)[1].trim(),
  password = raw.match(/^ControlPassword=(.*)$/m)[1].trim();
async function rpc(method, params) {
  const r = await fetch('http://127.0.0.1:26789/jsonrpc', {
    method: 'POST',
    headers: {
      Authorization: `Basic ${Buffer.from(`${username}:${password}`).toString('base64')}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ method, params, id: 1 }),
  });
  const data = await r.json();
  if (data.error) throw Error(`NZBGet ${method} rejected`);
  return data.result;
}
const settings = {
  MainDir: '/media/downloads',
  DestDir: '/media/downloads/completed',
  InterDir: '/media/downloads/intermediate',
  NzbDir: '/config/nzb',
  QueueDir: '/config/queue',
  TempDir: '/config/tmp',
  ScriptDir: '/config/scripts',
  LogFile: '/config/nzbget.log',
  'Server1.Active': 'yes',
  'Server1.Name': 'Local generated fixtures',
  'Server1.Host': 'nserv',
  'Server1.Port': '6791',
  'Server1.Username': '',
  'Server1.Password': '',
  'Server1.Encryption': 'no',
  'Server1.Connections': '2',
  'Server1.Retention': '0',
  'Category1.Name': 'movies',
  'Category2.Name': 'tv',
  'Category3.Name': 'music',
};
const existing = await rpc('loadconfig', []);
for (const [Name, Value] of Object.entries(settings)) {
  const entry = existing.find((e) => e.Name === Name);
  if (entry) entry.Value = Value;
  else existing.push({ Name, Value });
}
await rpc('saveconfig', [existing]);
await rpc('reload', []);
const prowlarr = arr('prowlarr', 29696, 1);
let indexer = (await prowlarr.api('indexer')).find(
  (i) => i.name === 'Generated fixtures',
);
if (!indexer) {
  const schema = await prowlarr.api('indexer/schema');
  indexer =
    schema.find((i) => i.name === 'Generic Newznab') ??
    schema.find((i) => i.implementation === 'Newznab');
  if (!indexer) throw Error('Missing Newznab schema');
  delete indexer.id;
  indexer.name = 'Generated fixtures';
  indexer.enable = true;
  indexer.appProfileId = 1;
  indexer.priority = 25;
  for (const field of indexer.fields) {
    if (field.name === 'baseUrl') field.value = 'http://indexer:8080';
    if (field.name === 'apiPath') field.value = '/api';
    if (field.name === 'apiKey') field.value = 'fixture';
  }
  indexer = await prowlarr.api('indexer', 'POST', indexer);
}
console.log(JSON.stringify({ prowlarr_indexer: indexer.id }));
for (const [kind, port, version, category] of [
  ['radarr', 27878, 3, 'movies'],
  ['sonarr', 28989, 3, 'tv'],
  ['lidarr', 28686, 1, 'music'],
]) {
  const client = arr(kind, port, version);
  if (
    !(await client.api('downloadclient')).some(
      (c) => c.name === 'Fixture NZBGet',
    )
  ) {
    const item = (await client.api('downloadclient/schema')).find(
      (c) => c.implementation === 'Nzbget',
    );
    if (!item) throw Error(`${kind} missing NZBGet schema`);
    delete item.id;
    item.name = 'Fixture NZBGet';
    item.enable = true;
    item.priority = 1;
    const values = {
      host: 'nzbget',
      port: 6789,
      useSsl: false,
      username,
      password,
      movieCategory: category,
      tvCategory: category,
      musicCategory: category,
      category,
      addPaused: false,
    };
    for (const field of item.fields)
      if (field.name in values) field.value = values[field.name];
    await client.api('downloadclient', 'POST', item);
  }
  if (
    !(await client.api('indexer')).some((c) => c.name === 'Generated fixtures')
  ) {
    const item = (await client.api('indexer/schema')).find(
      (c) => c.implementation === 'Newznab',
    );
    delete item.id;
    item.name = 'Generated fixtures';
    item.enableRss = false;
    item.enableAutomaticSearch = true;
    item.enableInteractiveSearch = true;
    item.priority = 25;
    const values = {
      baseUrl: `http://prowlarr:9696/${indexer.id}`,
      apiPath: '/api',
      apiKey: prowlarr.key,
      categories: [kind === 'radarr' ? 2000 : kind === 'sonarr' ? 5000 : 3000],
    };
    for (const field of item.fields)
      if (field.name in values) field.value = values[field.name];
    await client.api('indexer', 'POST', item);
  }
  // Synthetic files are intentionally very small; keep the fixture quality profile permissive.
  for (const definition of await client.api('qualitydefinition')) {
    if (definition.minSize !== 0) {
      definition.minSize = 0;
      await client.api(`qualitydefinition/${definition.id}`, 'PUT', definition);
    }
  }
  console.log(`${kind}: isolated indexer and downloader configured`);
}
