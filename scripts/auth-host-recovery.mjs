import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';

export async function qualifyHostRecovery(fixture, output) {
  let locked = false;
  try {
    fixture.compose(
      'run',
      '--rm',
      '--no-deps',
      'server',
      'recover-user',
      'admin',
    );
  } catch (error) {
    locked = /Resource temporarily unavailable|lock|already in use/i.test(
      String(error.stderr ?? error.message),
    );
  }
  assert(locked, 'Host recovery must refuse a running server');
  fixture.compose('stop', 'server');
  const result = fixture.compose(
    'run',
    '--rm',
    '--no-deps',
    'server',
    'recover-user',
    'admin',
  );
  const link = new URL(result.match(/https?:\/\/\S+#recovery=\S+/)?.[0]);
  const token = link.hash.slice('#recovery='.length);
  fixture.compose('up', '-d', '--wait', 'server');
  const api = (path, body) =>
    fetch(
      `http://127.0.0.1:${process.env.THELXINOE_TEST_HTTP_PORT}/api/v1${path}`,
      {
        method: 'POST',
        headers: {
          'X-Thelxinoe-Client': '1',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(body),
      },
    );
  const password = 'host recovery fixture passphrase';
  const info = await api('/auth/recovery/info', { token });
  assert.equal(info.status, 200);
  assert.equal((await info.json()).username, 'admin');
  const enrollment = await api('/auth/recovery/enroll', { token, password });
  assert.equal(enrollment.status, 200);
  assert.equal((await enrollment.json()).user, undefined);
  assert.equal(
    (await api('/auth/recovery/enroll', { token, password })).status,
    401,
  );
  const login = await api('/auth/login', {
    username: 'admin',
    password,
    transport: 'device',
  });
  assert.equal(login.status, 200);
  assert.equal((await login.json()).user.role, 'admin');
  writeFileSync(
    output,
    JSON.stringify(
      {
        passed: true,
        running_server_refused: true,
        one_use: true,
        role_preserved: true,
      },
      null,
      2,
    ),
  );
}
