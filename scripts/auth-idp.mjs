import { createServer } from 'node:http';
import {
  generateKeyPairSync,
  randomBytes,
  createHash,
  sign,
  timingSafeEqual,
} from 'node:crypto';

// A disposable protocol peer for real-server product E2Es. Nothing is persisted.
const port = Number(process.env.THELXINOE_TEST_OIDC_PORT ?? 18989);
const issuer = `http://127.0.0.1:${port}`;
const { privateKey, publicKey } = generateKeyPairSync('rsa', {
  modulusLength: 2048,
});
const jwk = {
  ...publicKey.export({ format: 'jwk' }),
  kid: 'fixture',
  use: 'sig',
  alg: 'RS256',
};
const codes = new Map();
const tokenGates = new Map();
const encode = (value) =>
  Buffer.from(JSON.stringify(value)).toString('base64url');
function jwt(claims) {
  const data = `${encode({ alg: 'RS256', kid: 'fixture', typ: 'JWT' })}.${encode(claims)}`;
  return `${data}.${sign('RSA-SHA256', Buffer.from(data), privateKey).toString('base64url')}`;
}
const server = createServer(async (req, res) => {
  res.setHeader('Cache-Control', 'no-store');
  const url = new URL(req.url, issuer);
  const providerIssuer = url.pathname.startsWith('/replacement/')
    ? `${issuer}/replacement`
    : issuer;
  const path = url.pathname.replace(/^\/replacement/, '');
  const json = (status, body) => {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(body));
  };
  if (url.pathname === '/health') return json(200, { ready: true });
  if (path === '/.well-known/openid-configuration')
    return json(200, {
      issuer: providerIssuer,
      authorization_endpoint: `${providerIssuer}/authorize`,
      token_endpoint: `${providerIssuer}/token`,
      jwks_uri: `${providerIssuer}/jwks`,
      response_types_supported: ['code'],
      subject_types_supported: ['public'],
      id_token_signing_alg_values_supported: ['RS256'],
      token_endpoint_auth_methods_supported: ['client_secret_basic'],
      scopes_supported: ['openid'],
      code_challenge_methods_supported: ['S256'],
    });
  if (path === '/jwks') return json(200, { keys: [jwk] });
  if (path === '/authorize' && req.method === 'GET') {
    const ticket = randomBytes(32).toString('hex');
    codes.set(ticket, {
      parameters: url.searchParams,
      expires: Date.now() + 60000,
    });
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    return res.end(
      `<html><body><form method="post"><input type="hidden" name="ticket" value="${ticket}"><label>Provider account<input name="subject" required></label><button>Continue</button></form></body></html>`,
    );
  }
  let body = '';
  for await (const chunk of req) {
    body += chunk;
    if (body.length > 4096) return json(413, {});
  }
  const input = new URLSearchParams(body);
  if (path === '/control/pause' && req.method === 'POST') {
    tokenGates.set(input.get('subject'), { waiting: false });
    return json(200, {});
  }
  if (path === '/control/status')
    return json(200, {
      waiting:
        tokenGates.get(url.searchParams.get('subject'))?.waiting ?? false,
    });
  if (path === '/control/release' && req.method === 'POST') {
    tokenGates.get(input.get('subject'))?.release?.();
    tokenGates.delete(input.get('subject'));
    return json(200, {});
  }
  if (path === '/authorize' && req.method === 'POST') {
    const pending = codes.get(input.get('ticket'));
    codes.delete(input.get('ticket'));
    if (!pending || pending.expires < Date.now())
      return json(400, { error: 'invalid_request' });
    const p = pending.parameters;
    if (
      p.get('client_id') !== 'thelxinoe-e2e' ||
      p.get('code_challenge_method') !== 'S256'
    )
      return json(400, { error: 'invalid_request' });
    const redirect = new URL(p.get('redirect_uri'));
    if (
      !['localhost', '127.0.0.1'].includes(redirect.hostname) ||
      redirect.pathname !== '/api/v1/auth/oidc/callback'
    )
      return json(400, { error: 'invalid_redirect_uri' });
    const code = randomBytes(32).toString('hex');
    codes.set(code, {
      parameters: p,
      subject: input.get('subject'),
      expires: Date.now() + 60000,
    });
    redirect.searchParams.set('code', code);
    redirect.searchParams.set('state', p.get('state'));
    res.writeHead(302, { Location: redirect.href });
    return res.end();
  }
  if (path === '/token' && req.method === 'POST') {
    const pending = codes.get(input.get('code'));
    codes.delete(input.get('code'));
    if (!pending || pending.expires < Date.now() || !pending.subject)
      return json(400, { error: 'invalid_grant' });
    const verifier = createHash('sha256')
      .update(input.get('code_verifier') ?? '')
      .digest('base64url');
    const expected = pending.parameters.get('code_challenge') ?? '';
    if (
      verifier.length !== expected.length ||
      !timingSafeEqual(Buffer.from(verifier), Buffer.from(expected)) ||
      input.get('redirect_uri') !== pending.parameters.get('redirect_uri') ||
      req.headers.authorization !==
        `Basic ${Buffer.from('thelxinoe-e2e:fixture-only').toString('base64')}`
    )
      return json(400, { error: 'invalid_grant' });
    const gate = tokenGates.get(pending.subject);
    if (gate) {
      await new Promise((resolve) => {
        gate.waiting = true;
        gate.release = resolve;
        res.once('close', resolve);
      });
      if (res.destroyed) return;
    }
    const time = Math.floor(Date.now() / 1000);
    return json(200, {
      token_type: 'Bearer',
      access_token: randomBytes(32).toString('hex'),
      expires_in: 300,
      id_token: jwt({
        iss:
          pending.subject === 'bad-issuer'
            ? providerIssuer + '/other'
            : providerIssuer,
        sub: pending.subject,
        aud:
          pending.subject === 'bad-audience' ? 'other-client' : 'thelxinoe-e2e',
        iat: time,
        exp: pending.subject === 'expired-token' ? time - 300 : time + 300,
        auth_time: pending.subject === 'stale-auth' ? time - 600 : time,
        nonce:
          pending.subject === 'bad-nonce'
            ? 'wrong-nonce'
            : pending.parameters.get('nonce'),
        email: 'same-email@example.test',
        role: 'admin',
      }),
    });
  }
  return json(404, { error: 'not_found' });
});
server.listen(port, '0.0.0.0');
for (const signal of ['SIGINT', 'SIGTERM'])
  process.on(signal, () => server.close());
