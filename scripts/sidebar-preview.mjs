import { createServer } from '../frontend/node_modules/vite/dist/node/index.js';
import {
  installSidebarFixture,
  sidebarArtwork,
} from '../tests/helpers/sidebar-fixture.ts';
import { freePort } from './ci-resources.mjs';
import { fileURLToPath } from 'node:url';

const handlers = [];
const initializers = [];
await installSidebarFixture({
  on() {},
  async addInitScript(callback, argument) {
    initializers.push(`(${callback.toString()})(${JSON.stringify(argument)});`);
  },
  async route(pattern, handler) {
    handlers.push({ pattern, handler });
  },
});
const port = process.env.THELXINOE_SIDEBAR_PREVIEW_PORT
  ? Number(process.env.THELXINOE_SIDEBAR_PREVIEW_PORT)
  : await freePort();
const server = await createServer({
  root: fileURLToPath(new URL('../frontend', import.meta.url)),
  server: { host: '127.0.0.1', port, strictPort: true, proxy: {} },
  plugins: [
    {
      name: 'sidebar-review-fixture',
      transformIndexHtml: () => [
        {
          tag: 'script',
          children: initializers.join('\n'),
          injectTo: 'head-prepend',
        },
      ],
      configureServer(vite) {
        vite.middlewares.use(async (req, res, next) => {
          const url = new URL(req.url, `http://127.0.0.1:${port}`);
          if (url.pathname.startsWith('/__sidebar-art/')) {
            res.setHeader('Content-Type', 'image/svg+xml');
            res.end(
              sidebarArtwork(Number(url.pathname.match(/\d+/)?.[0] ?? 0)),
            );
            return;
          }
          if (!url.pathname.startsWith('/api/')) return next();
          let body = '';
          for await (const chunk of req) body += chunk;
          const apiHandlers = handlers
            .filter((entry) => entry.pattern === '**/api/v1/**')
            .reverse();
          let index = 0;
          const route = {
            request: () => ({
              url: () => url.href,
              method: () => req.method,
              postDataJSON: () => (body ? JSON.parse(body) : null),
            }),
            fulfill({ json, status = 200 }) {
              res.statusCode = status;
              res.setHeader('Content-Type', 'application/json');
              res.end(JSON.stringify(json));
            },
            fallback: () => apiHandlers[index++]?.handler(route),
          };
          try {
            await route.fallback();
          } catch (error) {
            res.statusCode = 500;
            res.end(JSON.stringify({ error: { message: String(error) } }));
          }
        });
      },
    },
  ],
});
await server.listen();
console.log(`Sidebar review: http://127.0.0.1:${port}`);
const close = async () => {
  await server.close();
  process.exit(0);
};
process.on('SIGINT', close);
process.on('SIGTERM', close);
