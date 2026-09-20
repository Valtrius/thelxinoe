// Only a public connection screen is retained. API responses, media and the
// application shell always use the network so a release never serves stale code.
const CACHE = 'thelxinoe-connection-v1';
self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(CACHE).then((cache) => cache.add('/offline.html')),
  );
  self.skipWaiting();
});
self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) {
        if (key.startsWith('thelxinoe-connection-') && key !== CACHE)
          await caches.delete(key);
      }
      await self.clients.claim();
    })(),
  );
});
self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (
    event.request.mode === 'navigate' &&
    url.origin === self.location.origin &&
    url.pathname === '/'
  ) {
    event.respondWith(
      fetch(event.request).catch(
        async () => (await caches.match('/offline.html')) || Response.error(),
      ),
    );
  }
});
