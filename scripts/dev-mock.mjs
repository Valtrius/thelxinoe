import { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { WebSocketServer } from 'ws';
import { createReadStream, existsSync, statSync } from 'node:fs';
import { artwork } from './density-fixtures.mjs';
import { createMockState, installMockApi } from './mock/api.mjs';
import { prepareSample, samplePath } from './mock/playback.mjs';
import { mockUiPlugin } from './mock/build.mjs';
import { freePort } from './ci-resources.mjs';
import { controlScenario, scenarioView } from './mock/scenarios.mjs';

const requireFrontend = createRequire(
  new URL('../frontend/package.json', import.meta.url),
);
const { build, preview } = await import(
  pathToFileURL(requireFrontend.resolve('vite')).href
);
const root = fileURLToPath(new URL('../frontend', import.meta.url));
const configFile = fileURLToPath(
  new URL('../frontend/vite.config.ts', import.meta.url),
);
// Keep the mock-only artwork adapter out of the production distribution.
const port = Number(process.env.THELXINOE_MOCK_PORT ?? (await freePort()));
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw Error('Invalid THELXINOE_MOCK_PORT');
const outDir = fileURLToPath(
  new URL(`../.local/mock/${port}/dist`, import.meta.url),
);
const routes = [];
let state = createMockState();
let cursor = 0;
const unhandled = [];
const sockets = new WebSocketServer({ noServer: true });
const events = {
  cursor: () => cursor,
  emit(kind, payload) {
    const event = JSON.stringify({ id: ++cursor, kind, payload });
    for (const client of sockets.clients)
      if (client.readyState === 1) client.send(event);
  },
};
const fixturePage = {
  on() {},
  async addInitScript() {},
  async route(pattern, handle) {
    const source = pattern
      .split('**')
      .map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('.*');
    routes.unshift({ pattern: new RegExp(`^${source}$`), handle });
  },
};
await fixturePage.route('**/mock-art/**', (route) => {
  const [, kind, index] = new URL(route.request().url()).pathname
    .split('/')
    .slice(1);
  return route.fulfill({
    contentType: 'image/svg+xml',
    body: artwork(kind, Number(index)),
  });
});
await installMockApi(fixturePage, () => state, events);
try {
  await prepareSample();
} catch (error) {
  console.warn(`Sample playback unavailable: ${error.message}`);
}

await build({
  root,
  configFile,
  build: { outDir, emptyOutDir: true },
  plugins: [mockUiPlugin()],
});
const server = await preview({
  root,
  configFile,
  build: { outDir },
  preview: {
    host: '127.0.0.1',
    port,
    strictPort: true,
    open: false,
    proxy: {},
  },
  plugins: [
    {
      name: 'disposable-ui-preview',
      configurePreviewServer(vite) {
        // Run before the SPA fallback and never forward requests to real services.
        vite.middlewares.use(async (req, res, next) => {
          if (req.url?.startsWith('/mock-media/')) {
            if (req.url === '/mock-media/english.vtt') {
              res.writeHead(200, { 'Content-Type': 'text/vtt' });
              return res.end(
                'WEBVTT\n\n00:00:00.000 --> 00:00:45.000\nLocal preview media\n\n00:00:45.000 --> 00:01:30.000\nThelxinoe playback preview\n',
              );
            }
            if (req.url !== '/mock-media/sample.mp4') {
              res.writeHead(404);
              return res.end();
            }
            if (!existsSync(samplePath)) {
              res.writeHead(503, { 'Content-Type': 'text/plain' });
              return res.end(
                'Install FFmpeg and restart dev:mock to enable playback.',
              );
            }
            const size = statSync(samplePath).size;
            const range = req.headers.range?.match(/^bytes=(\d+)-(\d*)$/);
            const start = range ? Number(range[1]) : 0;
            const end = range?.[2]
              ? Math.min(Number(range[2]), size - 1)
              : size - 1;
            if (start > end || start >= size) {
              res.writeHead(416, { 'Content-Range': `bytes */${size}` });
              return res.end();
            }
            res.writeHead(range ? 206 : 200, {
              'Content-Type': 'video/mp4',
              'Accept-Ranges': 'bytes',
              'Content-Length': end - start + 1,
              ...(range
                ? { 'Content-Range': `bytes ${start}-${end}/${size}` }
                : {}),
            });
            if (req.method === 'HEAD') return res.end();
            return createReadStream(samplePath, { start, end }).pipe(res);
          }
          if (/^\/(movie|tv)\/\d+\/?(?:\?.*)?$/.test(req.url ?? '')) {
            res.writeHead(302, {
              Location: `/#discover${req.url.replace(/\/$/, '')}`,
            });
            return res.end();
          }
          if (req.url?.startsWith('/__mock/')) {
            if (req.url === '/__mock/services') {
              try {
                if (req.method === 'POST') {
                  let raw = '';
                  for await (const chunk of req) raw += chunk;
                  controlScenario(state, JSON.parse(raw));
                  events.emit('services.changed', {});
                } else if (req.method !== 'GET') {
                  res.writeHead(405);
                  return res.end();
                }
                res.writeHead(200, {
                  'Content-Type': 'application/json',
                  'Cache-Control': 'no-store',
                });
                return res.end(JSON.stringify(scenarioView(state)));
              } catch (error) {
                res.writeHead(400, { 'Content-Type': 'application/json' });
                return res.end(JSON.stringify({ error: error.message }));
              }
            }
            if (req.url === '/__mock/viewer') {
              if (req.method === 'POST') {
                let raw = '';
                for await (const chunk of req) raw += chunk;
                const { role } = JSON.parse(raw || '{}');
                if (!['admin', 'user'].includes(role)) {
                  res.writeHead(400, { 'Content-Type': 'application/json' });
                  return res.end(
                    JSON.stringify({ error: 'Choose the admin or user view' }),
                  );
                }
                state.viewer = role;
              } else if (req.method !== 'GET') {
                res.writeHead(405);
                return res.end();
              }
              res.writeHead(200, {
                'Content-Type': 'application/json',
                'Cache-Control': 'no-store',
              });
              return res.end(JSON.stringify({ role: state.viewer }));
            }
            if (req.url === '/__mock/reset' && req.method === 'POST') {
              state = createMockState();
              unhandled.length = 0;
              events.emit('server.reconnected', {});
            }
            if (req.url === '/__mock/auth') {
              state.signedIn = true;
              state.auth.oidc = 'Preview identity';
              res.writeHead(302, {
                Location: '/?auth_notice=oidc-linked#settings/account',
              });
              return res.end();
            }
            if (req.url.startsWith('/__mock/connect/')) {
              const platform = req.url.split('/').at(-1);
              state.connected[platform] = true;
              res.writeHead(302, { Location: '/#settings/online' });
              return res.end();
            }
            res.writeHead(200, {
              'Content-Type': 'application/json',
              'Cache-Control': 'no-store',
            });
            return res.end(
              JSON.stringify(
                req.url === '/__mock/diagnostics'
                  ? { unhandled, clients: sockets.clients.size }
                  : { ready: true },
              ),
            );
          }
          if (
            req.url?.startsWith('/services/') ||
            req.url?.startsWith('/service-access/')
          ) {
            res.writeHead(200, { 'Content-Type': 'text/html' });
            return res.end(
              '<!doctype html><title>Mock service</title><p>Service containers are simulated in this preview.</p>',
            );
          }
          if (
            !req.url?.startsWith('/api/v1/') &&
            !req.url?.startsWith('/mock-art/')
          )
            return next();
          try {
            const url = new URL(req.url, `http://127.0.0.1:${port}`).href;
            let raw = '';
            for await (const chunk of req) raw += chunk;
            const body =
              raw && req.headers['content-type']?.includes('application/json')
                ? JSON.parse(raw)
                : null;
            const request = {
              url: () => url,
              method: () => req.method,
              postDataJSON: () => body,
            };
            const matching = routes.filter((route) => route.pattern.test(url));
            async function dispatch(index) {
              if (!matching[index]) {
                unhandled.push(`${req.method} ${req.url}`);
                res.writeHead(404, { 'Content-Type': 'application/json' });
                return res.end(
                  JSON.stringify({
                    error: { message: `No mock for ${req.method} ${req.url}` },
                  }),
                );
              }
              return matching[index].handle({
                request: () => request,
                fallback: () => dispatch(index + 1),
                fulfill({
                  json,
                  body: responseBody,
                  status = 200,
                  contentType,
                  headers = {},
                }) {
                  if (status >= 400 && !headers['x-thelxinoe-mock-error'])
                    unhandled.push(
                      `${req.method} ${req.url}: ${json?.error?.message ?? status}`,
                    );
                  res.writeHead(status, {
                    'Content-Type': contentType ?? 'application/json',
                    'Cache-Control': 'no-store',
                    ...headers,
                  });
                  res.end(
                    json === undefined ? responseBody : JSON.stringify(json),
                  );
                },
              });
            }
            await dispatch(0);
          } catch (error) {
            console.error(error);
            unhandled.push(`${req.method} ${req.url}: ${error.message}`);
            if (!res.headersSent) res.writeHead(500);
            res.end('Mock request failed');
          }
        });
      },
    },
  ],
});
server.httpServer.on('upgrade', (request, socket, head) => {
  if (!request.url?.startsWith('/api/v1/events?')) return socket.destroy();
  sockets.handleUpgrade(request, socket, head, (client) =>
    sockets.emit('connection', client, request),
  );
});
console.log(`Mock preview: http://127.0.0.1:${port}/#home`);
console.log('Switch between the admin and user views at the bottom right.');
console.log(
  `Movie: http://127.0.0.1:${port}/movie/11 | TV: http://127.0.0.1:${port}/tv/22`,
);
console.log(
  'Changes last until restart. POST /__mock/reset restores the seed data.',
);
for (const signal of ['SIGINT', 'SIGTERM'])
  process.on(signal, async () => {
    for (const socket of sockets.clients) socket.terminate();
    sockets.close();
    await new Promise((resolve) => server.httpServer.close(resolve));
    process.exit(0);
  });
