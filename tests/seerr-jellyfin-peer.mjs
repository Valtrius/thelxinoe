import { createServer } from 'node:http';

// Test-only Jellyfin sign-in so an external Seerr can be initialized offline the way an
// administrator would set it up, giving the transfer an existing administrator account.
const server = 'fixture-jellyfin';
const token = 'f'.repeat(64);
const user = {
  Id: 'fixture-administrator',
  Name: 'fixture-admin',
  ServerId: server,
  Policy: { IsAdministrator: true },
  Configuration: { GroupedFolders: [] },
};
const json = (response, status, value) => {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(value === undefined ? '' : JSON.stringify(value));
};
createServer((request, response) => {
  const path = new URL(request.url, 'http://peer').pathname.replace(/\/+$/, '');
  const route = `${request.method} ${path}`;
  if (route === 'GET /System/Info/Public')
    return json(response, 200, {
      Id: server,
      ServerName: 'Fixture Jellyfin',
      Version: '10.10.7',
      ProductName: 'Jellyfin Server',
    });
  if (route === 'POST /Users/AuthenticateByName')
    return json(response, 200, {
      User: user,
      AccessToken: token,
      ServerId: server,
    });
  if (route === 'GET /System/Info')
    return json(response, 200, {
      Id: server,
      ServerName: 'Fixture Jellyfin',
      Version: '10.10.7',
    });
  if (route === 'POST /Auth/Keys') return json(response, 204);
  if (route === 'GET /Auth/Keys')
    return json(response, 200, {
      Items: [{ AppName: 'Seerr', AccessToken: token }],
    });
  if (route === 'GET /Users/Me') return json(response, 200, user);
  return json(response, 404, {});
}).listen(8096);
