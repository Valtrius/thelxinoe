import { test, expect, type BrowserContext, type Page } from '@playwright/test';
import { createHmac } from 'node:crypto';

const password = 'authentication fixture passphrase';
const headers = { 'X-Thelxinoe-Client': '1' };
function otp(secret: string, offset = 0) {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  const bits = [...secret]
    .map((c) => alphabet.indexOf(c).toString(2).padStart(5, '0'))
    .join('');
  const bytes = Buffer.from(bits.match(/.{8}/g)!.map((b) => parseInt(b, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000) + offset));
  const hash = createHmac('sha1', bytes).update(counter).digest();
  const index = hash[hash.length - 1] & 15;
  return ((hash.readUInt32BE(index) & 0x7fffffff) % 1000000)
    .toString()
    .padStart(6, '0');
}
async function post(
  page: Page,
  path: string,
  data: unknown = {},
  requestHeaders = headers,
) {
  return page.request.post(`/api/v1${path}`, { headers: requestHeaders, data });
}
async function login(page: Page, username: string, value = password) {
  await page.goto('/#home');
  await page.getByLabel('Username', { exact: true }).fill(username);
  await page.getByLabel('Password', { exact: true }).fill(value);
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
}
async function authenticator(context: BrowserContext, page: Page) {
  const cdp = await context.newCDPSession(page);
  await cdp.send('WebAuthn.enable');
  await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return cdp;
}

test('real password, TOTP, passkey, remembered device, client access and administrator recovery', async ({
  browser,
}, info) => {
  test.skip(
    !process.env.THELXINOE_PROXY_TEST,
    'Requires the isolated real server and HTTPS proxy',
  );
  test.setTimeout(180000);
  const adminContext = await browser.newContext({
    ignoreHTTPSErrors: true,
    baseURL: info.project.use.baseURL,
  });
  const admin = await adminContext.newPage();
  await login(admin, 'admin', 'test-only long passphrase');
  await expect(
    admin.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  const username = `auth${Date.now()}`;
  const created = await post(admin, '/users', {
    username,
    password,
    role: 'user',
  });
  expect(created.ok()).toBeTruthy();
  const userId = (await created.json()).id;
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    baseURL: info.project.use.baseURL,
  });
  const page = await context.newPage();
  await authenticator(context, page);
  await login(page, username);
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  await page.goto('/#settings/account');
  await page.getByRole('button', { name: 'Add passkey', exact: true }).click();
  await expect(page.getByText('Passkey added', { exact: true })).toBeVisible();
  const start = await post(page, '/me/auth/totp/start');
  expect(start.ok()).toBeTruthy();
  const { secret } = await start.json();
  expect(
    (await post(page, '/me/auth/totp/confirm', { code: otp(secret) })).ok(),
  ).toBeTruthy();
  await post(page, '/auth/logout');
  await login(page, username);
  await expect(
    page.getByLabel('Authentication code', { exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel('Remember this device')).not.toBeChecked();
  expect((await page.request.get('/api/v1/auth/me')).status()).toBe(401);
  await page.getByLabel('Authentication code', { exact: true }).fill('000000');
  await page.getByRole('button', { name: 'Verify code', exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  // Use the next accepted step, distinct from enrollment: replays must fail.
  await page
    .getByLabel('Authentication code', { exact: true })
    .fill(otp(secret, 1));
  await page.getByLabel('Remember this device').check();
  await page.getByRole('button', { name: 'Verify code', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  const sessions = await (
    await page.request.get('/api/v1/auth/sessions')
  ).json();
  expect(
    sessions.items.find((s: { id: string }) => s.id === sessions.current)
      .remembered,
  ).toBe(true);
  const remembered = (await context.cookies()).find(
    (c) => c.name === 'thelxinoe_remember',
  );
  expect(remembered).toMatchObject({
    httpOnly: true,
    secure: true,
    sameSite: 'Strict',
  });
  const appPasswordResponse = await post(page, '/me/auth/client-passwords', {
    name: 'Living room',
  });
  expect(appPasswordResponse.ok()).toBeTruthy();
  const appPassword = await appPasswordResponse.json();
  const jellyfin = {
    'X-Emby-Authorization':
      'MediaBrowser Client="auth E2E", Device="TV", DeviceId="auth-e2e", Version="1"',
  };
  expect(
    (
      await page.request.post('/Users/AuthenticateByName', {
        headers: jellyfin,
        data: { Username: username, Pw: password },
      })
    ).status(),
  ).toBe(401);
  const client = await page.request.post('/Users/AuthenticateByName', {
    headers: jellyfin,
    data: { Username: username, Pw: appPassword.password },
  });
  expect(client.ok()).toBeTruthy();
  const clientToken = (await client.json()).AccessToken;
  expect(
    (
      await page.request.get('/api/v1/auth/me', {
        headers: { Authorization: `Bearer ${clientToken}` },
      })
    ).status(),
  ).toBe(401);
  await post(page, '/auth/logout');
  await login(page, username);
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  expect(
    (
      await post(page, '/me/auth/client-passwords', { name: 'Unverified' })
    ).status(),
  ).toBe(403);
  await page.goto('/#settings/account');
  await page
    .getByLabel('New password', { exact: true })
    .fill('canceled password change');
  await page
    .getByLabel('Confirm new password', { exact: true })
    .fill('canceled password change');
  await page
    .getByRole('button', { name: 'Change password', exact: true })
    .click();
  const proofDialog = page.getByRole('dialog', {
    name: 'Verify your identity',
  });
  await expect(proofDialog).toBeVisible();
  await proofDialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(proofDialog).not.toBeVisible();
  await expect(page.getByLabel('New password', { exact: true })).toHaveValue(
    'canceled password change',
  );
  await expect(page.getByText(/Password changed\./)).not.toBeVisible();
  await page
    .getByRole('button', { name: 'Verify with passkey', exact: true })
    .click();
  await expect(
    page.getByText('Identity verified', { exact: true }),
  ).toBeVisible();
  expect(
    (
      await page.request.delete(
        `/api/v1/me/auth/client-passwords/${appPassword.id}`,
        { headers },
      )
    ).ok(),
  ).toBeTruthy();
  expect(
    (
      await page.request.get('/Users/Me', {
        headers: { 'X-Emby-Token': clientToken },
      })
    ).status(),
  ).toBe(401);
  const devices = await (await page.request.get('/api/v1/auth/devices')).json();
  expect(devices.items).toHaveLength(1);
  expect(
    (
      await page.request.delete(`/api/v1/auth/devices/${devices.items[0].id}`, {
        headers,
      })
    ).ok(),
  ).toBeTruthy();
  expect((await page.request.get('/api/v1/auth/me')).status()).toBe(401);
  await login(page, username);
  await expect(
    page.getByLabel('Authentication code', { exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Back to sign in', exact: true })
    .click();
  await page.getByLabel('Username', { exact: true }).fill(username);
  await page
    .getByRole('button', { name: 'Sign in with passkey', exact: true })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  expect(
    (await page.request.delete('/api/v1/me/auth/password', { headers })).ok(),
  ).toBeTruthy();
  const methods = await (await page.request.get('/api/v1/me/auth')).json();
  expect(methods.password).toBe(false);
  expect(
    (
      await page.request.delete(
        `/api/v1/me/auth/passkeys/${methods.passkeys[0].id}`,
        { headers },
      )
    ).status(),
  ).toBe(409);
  expect(
    (
      await admin.request.put(`/api/v1/users/${userId}`, {
        headers,
        data: { role: 'user', password: 'legacy reset passphrase' },
      })
    ).status(),
  ).toBe(400);
  const recovery = await post(admin, `/users/${userId}/recovery`);
  expect(recovery.ok()).toBeTruthy();
  const { url } = await recovery.json();
  expect((await page.request.get('/api/v1/auth/me')).status()).toBe(401);
  await page.goto(url);
  await expect(
    page.getByRole('heading', { name: 'Recover account', exact: true }),
  ).toBeVisible();
  await page.getByLabel('New password', { exact: true }).fill(password);
  await page.getByLabel('Confirm password', { exact: true }).fill(password);
  await page
    .getByRole('button', { name: 'Save sign-in method', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Sign in', exact: true }),
  ).toBeVisible();
  const token = new URL(url).hash.slice('#recovery='.length);
  expect(
    (await post(page, '/auth/recovery/enroll', { token, password })).status(),
  ).toBe(401);
  await login(page, username);
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  await page.goto('/#settings/account');
  await expect(
    page.getByLabel('Current password', { exact: true }),
  ).not.toBeVisible();
  await page
    .getByLabel('New password', { exact: true })
    .fill('replacement fixture passphrase');
  await page
    .getByLabel('Confirm new password', { exact: true })
    .fill('replacement fixture passphrase');
  await page
    .getByRole('button', { name: 'Change password', exact: true })
    .click();
  await expect(page.getByText(/Password changed\./)).toBeVisible();
  await post(page, '/auth/logout');
  await login(page, username, 'replacement fixture passphrase');
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  await page.screenshot({
    path: info.outputPath('recovered-account.png'),
    fullPage: true,
  });
  await info.attach('authentication decisions', {
    body: Buffer.from(
      JSON.stringify({
        username,
        passkey: true,
        totp: true,
        remembered: true,
        recovery: true,
      }),
    ),
    contentType: 'application/json',
  });
  await context.close();
  await adminContext.close();
});

test('generic OIDC explicitly links accounts, verifies identities and binds native browser exchanges', async ({
  browser,
}, info) => {
  test.skip(
    !process.env.THELXINOE_PROXY_TEST,
    'Requires the real server, HTTPS proxy and disposable OIDC provider',
  );
  const context = await browser.newContext({
    ignoreHTTPSErrors: true,
    baseURL: info.project.use.baseURL,
  });
  const page = await context.newPage();
  await login(page, 'admin', 'test-only long passphrase');
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  const issuer = `http://127.0.0.1:${process.env.THELXINOE_TEST_OIDC_PORT ?? '18989'}`;
  const configured = await post(page, '/admin/auth/oidc', {
    discovery_url: `${issuer}/.well-known/openid-configuration`,
    client_id: 'thelxinoe-e2e',
    client_secret: 'fixture-only',
    label: 'Fixture SSO',
  });
  expect(configured.ok()).toBeTruthy();
  const link = await post(page, '/auth/oidc/start', {
    purpose: 'link',
    return_to: '/#settings/account',
  });
  await page.goto((await link.json()).url);
  await page.getByLabel('Provider account').fill('admin-subject');
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(
    page.getByText('OIDC account linked', { exact: true }),
  ).toBeVisible();
  await post(page, '/auth/logout');
  await page.goto('/#home');
  await page.getByRole('button', { name: 'Fixture SSO', exact: true }).click();
  await page.getByLabel('Provider account').fill('unlinked-subject');
  await page.getByRole('button', { name: 'Continue' }).click();
  await expect(page.getByRole('alert')).toContainText('not linked');
  for (const subject of [
    'bad-nonce',
    'bad-audience',
    'bad-issuer',
    'expired-token',
    'stale-auth',
  ]) {
    await page
      .getByRole('button', { name: 'Fixture SSO', exact: true })
      .click();
    await page.getByLabel('Provider account').fill(subject);
    await page.getByRole('button', { name: 'Continue' }).click();
    await expect(page.getByRole('alert')).toContainText(
      subject === 'stale-auth' ? 'fresh sign-in' : 'verification failed',
    );
    expect((await page.request.get('/api/v1/auth/me')).status()).toBe(401);
  }
  await page.getByRole('button', { name: 'Fixture SSO', exact: true }).click();
  await page.getByLabel('Provider account').fill('admin-subject');
  await page.getByRole('button', { name: 'Continue' }).click();
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Home', exact: true }),
  ).toBeVisible();
  const handoff = await post(page, '/auth/desktop/start', {
    device_name: 'Native fixture',
  });
  const started = await handoff.json();
  await page.goto(started.url);
  await expect(
    page.getByRole('heading', { name: 'Connect desktop', exact: true }),
  ).toBeVisible();
  await page
    .getByRole('button', { name: 'Approve desktop', exact: true })
    .click();
  await expect(
    page.getByText('Desktop approved', { exact: true }),
  ).toBeVisible();
  expect(
    (
      await post(page, '/auth/desktop/exchange', {
        request: started.request,
        secret: 'wrong',
      })
    ).status(),
  ).toBe(401);
  const exchanged = await post(page, '/auth/desktop/exchange', {
    request: started.request,
    secret: started.secret,
  });
  expect(exchanged.ok()).toBeTruthy();
  const { token } = await exchanged.json();
  expect(
    (
      await page.request.get('/api/v1/auth/me', {
        headers: { Authorization: `Bearer ${token}` },
      })
    ).ok(),
  ).toBeTruthy();
  expect(
    (
      await post(page, '/auth/desktop/exchange', {
        request: started.request,
        secret: started.secret,
      })
    ).status(),
  ).toBe(401);
  const verifiedDesktop = await post(
    page,
    '/auth/desktop/start',
    {
      purpose: 'verify',
      device_name: 'Native verification fixture',
    },
    { ...headers, Authorization: `Bearer ${token}` },
  );
  const verificationRequest = await verifiedDesktop.json();
  await page.goto(verificationRequest.url);
  await page
    .getByRole('button', { name: 'Approve desktop', exact: true })
    .click();
  await expect(
    page.getByText('Desktop approved', { exact: true }),
  ).toBeVisible();
  const verifiedExchange = await post(
    page,
    '/auth/desktop/exchange',
    {
      request: verificationRequest.request,
      secret: verificationRequest.secret,
    },
    { ...headers, Authorization: `Bearer ${token}` },
  );
  expect((await verifiedExchange.json()).verified).toBe(true);
  expect(
    (
      await post(
        page,
        '/auth/desktop/exchange',
        {
          request: verificationRequest.request,
          secret: verificationRequest.secret,
        },
        { ...headers, Authorization: `Bearer ${token}` },
      )
    ).status(),
  ).toBe(401);
  const canceled = await (
    await post(page, '/auth/desktop/start', { device_name: 'Canceled fixture' })
  ).json();
  expect(
    (
      await post(page, '/auth/desktop/cancel', {
        request: canceled.request,
        secret: canceled.secret,
      })
    ).ok(),
  ).toBeTruthy();
  expect(
    (
      await post(page, '/auth/desktop/exchange', {
        request: canceled.request,
        secret: canceled.secret,
      })
    ).status(),
  ).toBe(401);
  expect(
    (await page.request.delete('/api/v1/me/auth/password', { headers })).ok(),
  ).toBe(true);
  expect(
    (
      await page.request.delete('/api/v1/admin/auth/oidc', { headers })
    ).status(),
  ).toBe(409);
  expect(
    (
      await page.request.put('/api/v1/me/password', {
        headers,
        data: { new_password: 'test-only long passphrase' },
      })
    ).ok(),
  ).toBe(true);
  await page.screenshot({
    path: info.outputPath('desktop-approved.png'),
    fullPage: true,
  });
  await context.close();
});
