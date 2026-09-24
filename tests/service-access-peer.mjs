// Hostile-response fixture for the gateway boundary, never a normal UI substitute.
import { createServer } from 'node:http';
createServer((request, response) => {
  const path = new URL(request.url, 'http://fixture').pathname;
  response.setHeader('Content-Type', 'application/json');
  if (path === '/services/radarr/api/v3/system/status') {
    response.end(
      JSON.stringify({
        appName: 'Radarr',
        urlBase: '/services/radarr',
        version: 'fixture',
      }),
    );
  } else if (path === '/services/radarr/api/v3/rootfolder') {
    response.end('[]');
  } else if (path === '/services/radarr/headers') {
    response.end(JSON.stringify(request.headers));
  } else if (path === '/services/radarr/cookies') {
    response.setHeader('Set-Cookie', [
      'native=fixture; Domain=example.invalid; Path=/; HttpOnly',
      'thelxinoe_session=overwrite; Path=/',
    ]);
    response.setHeader('Service-Worker-Allowed', '/');
    response.end('{}');
  } else if (path === '/services/radarr/redirect') {
    response.writeHead(302, { Location: 'https://example.invalid/collect' });
    response.end();
  } else if (
    path === '/services/radarr/range' &&
    request.headers.range === 'bytes=0-4'
  ) {
    response.writeHead(206, {
      'Content-Type': 'application/octet-stream',
      'Content-Range': 'bytes 0-4/11',
      'Content-Disposition': 'attachment; filename="fixture.bin"',
    });
    response.end('range');
  } else if (path === '/services/radarr/stream') {
    response.writeHead(200, { 'Content-Type': 'text/event-stream' });
    const timer = setInterval(() => response.write('data: fixture\n\n'), 100);
    request.on('close', () => clearInterval(timer));
  } else if (path === '/services/radarr/pending') {
    const timer = setTimeout(() => response.end('{}'), 60000);
    response.on('close', () => clearTimeout(timer));
  } else {
    response.writeHead(404);
    response.end('{}');
  }
}).listen(7878, '0.0.0.0');
