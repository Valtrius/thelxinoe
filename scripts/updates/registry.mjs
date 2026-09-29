import { createServer, request } from 'node:http';
import { createHash, randomBytes } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { once } from 'node:events';
import { readFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

function registryProxy(lab, slow) {
  return createServer((incoming, outgoing) => {
    const upstream = request(
      {
        hostname: lab.registryHost ?? '127.0.0.1',
        port: lab.registryBackendPort,
        method: incoming.method,
        path: incoming.url,
        headers: incoming.headers,
      },
      async (response) => {
        outgoing.writeHead(response.statusCode, response.headers);
        outgoing.on('close', () => response.destroy());
        if (
          !slow() ||
          incoming.method !== 'GET' ||
          !incoming.url.includes('/blobs/')
        ) {
          response.pipe(outgoing);
          return;
        }
        try {
          for await (const chunk of response) {
            for (let offset = 0; offset < chunk.length; offset += 64 * 1024) {
              if (outgoing.destroyed) return;
              if (!outgoing.write(chunk.subarray(offset, offset + 64 * 1024)))
                await once(outgoing, 'drain');
              await delay(100);
            }
          }
          outgoing.end();
        } catch {
          outgoing.destroy();
        }
      },
    );
    upstream.on('error', () => {
      if (!outgoing.headersSent) outgoing.writeHead(502);
      outgoing.end();
    });
    incoming.pipe(upstream);
  }).listen(lab.registryPort, lab.registryBind ?? '127.0.0.1');
}

if (process.argv[2] === 'serve')
  registryProxy(
    {
      registryHost: process.argv[3],
      registryBackendPort: 5000,
      registryPort: 5000,
      registryBind: '0.0.0.0',
    },
    () =>
      JSON.parse(readFileSync(process.argv[4], 'utf8')).mode ===
      'slow-download',
  );

const digest = (bytes) =>
  `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
const imageType = 'application/vnd.oci.image.manifest.v1+json';
// Registry-only content has never entered the Docker/BuildKit cache. This makes
// slow-download exercise actual engine byte progress without pruning host data.
export async function downloadFixture(lab, reference) {
  const [name, source] = reference.split('@');
  const repository = name.slice(name.indexOf('/') + 1);
  const origin = `http://127.0.0.1:${lab.registryPort}`;
  const base = `${origin}/v2/${repository}`;
  const checked = async (url, options) => {
    const response = await fetch(url, options);
    if (!response.ok)
      throw Error(
        `Lab registry HTTP ${response.status}: ${new URL(url).pathname}`,
      );
    return response;
  };
  const manifestAt = async (hash) =>
    (
      await checked(`${base}/manifests/${hash}`, {
        headers: {
          Accept: `${imageType}, application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json`,
        },
      })
    ).json();
  let manifest = await manifestAt(source);
  if (manifest.manifests) {
    const linux = manifest.manifests.find(
      (item) =>
        item.platform?.os === 'linux' && item.platform.architecture === 'amd64',
    );
    if (!linux) throw Error('Lab image has no Linux amd64 manifest');
    manifest = await manifestAt(linux.digest);
  }
  const config = await (
    await checked(`${base}/blobs/${manifest.config.digest}`)
  ).json();
  const payload = randomBytes(8 * 1024 * 1024);
  const header = Buffer.alloc(512);
  header.write('thelxinoe-update-lab.bin');
  header.write('0000644\0', 100);
  header.write('0000000\0', 108);
  header.write('0000000\0', 116);
  header.write(payload.length.toString(8).padStart(11, '0') + '\0', 124);
  header.write('00000000000\0', 136);
  header.fill(32, 148, 156);
  header.write('0', 156);
  header.write('ustar\0', 257);
  header.write('00', 263);
  header.write(
    [...header]
      .reduce((sum, byte) => sum + byte, 0)
      .toString(8)
      .padStart(6, '0') + '\0 ',
    148,
  );
  const tar = Buffer.concat([header, payload, Buffer.alloc(1024)]);
  const blob = gzipSync(tar);
  const upload = async (bytes) => {
    const hash = digest(bytes);
    const begin = await checked(`${base}/blobs/uploads/`, { method: 'POST' });
    const location = new URL(begin.headers.get('location'), origin);
    const url = new URL(location.pathname + location.search, origin);
    url.searchParams.set('digest', hash);
    await checked(url, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/octet-stream' },
      body: bytes,
    });
    return hash;
  };
  const layer = await upload(blob);
  config.rootfs.diff_ids.push(digest(tar));
  config.history ??= [];
  config.history.push({ created_by: 'Thelxinoe update lab download fixture' });
  const configBytes = Buffer.from(JSON.stringify(config));
  manifest.config = {
    ...manifest.config,
    digest: await upload(configBytes),
    size: configBytes.length,
  };
  manifest.layers.push({
    mediaType: 'application/vnd.oci.image.layer.v1.tar+gzip',
    digest: layer,
    size: blob.length,
  });
  const bytes = Buffer.from(JSON.stringify(manifest));
  await checked(`${base}/manifests/download-fixture`, {
    method: 'PUT',
    headers: { 'Content-Type': manifest.mediaType ?? imageType },
    body: bytes,
  });
  return { reference: `${name}@${digest(bytes)}` };
}
