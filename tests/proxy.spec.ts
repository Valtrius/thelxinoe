import { test, expect } from '@playwright/test';

test('HTTPS setup, secure login, event replay, CSRF rejection and device revocation', async ({
  page,
  context,
  request,
}) => {
  test.skip(
    !process.env.THELXINOE_PROXY_TEST,
    'Requires isolated Compose proxy fixture',
  );
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).waitFor();
  const setup = await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .isVisible();
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('test-only long passphrase');
  if (setup)
    await page
      .getByLabel('Confirm password', { exact: true })
      .fill('test-only long passphrase');
  await page
    .getByRole('button', {
      name: setup ? 'Create your server' : 'Sign in',
      exact: true,
    })
    .click();
  await expect(
    page.getByRole('heading', { name: 'Discover', exact: true }),
  ).toBeVisible();
  await expect(page.getByText('Connected', { exact: true })).toBeVisible();
  const cookies = await context.cookies();
  expect(cookies.find((c) => c.name === 'thelxinoe_session')).toMatchObject({
    secure: true,
    httpOnly: true,
    sameSite: 'Strict',
  });
  const headers = { 'X-Thelxinoe-Client': '1' };
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page
    .getByRole('navigation', { name: 'Settings navigation' })
    .getByRole('button', { name: 'Server', exact: true })
    .click();
  await expect(page.getByText('Connected', { exact: true })).toHaveCount(2);
  const events = await page.evaluate(async () => {
    const ticket = async () =>
      (
        await (
          await fetch('/api/v1/auth/event-ticket', {
            method: 'POST',
            headers: { 'X-Thelxinoe-Client': '1' },
          })
        ).json()
      ).ticket;
    const receive = (credential: string, since: number) =>
      new Promise<{ id: number; kind: string }>((resolve, reject) => {
        const socket = new WebSocket(
          `wss://${location.host}/api/v1/events?ticket=${credential}&since=${since}`,
        );
        const timeout = setTimeout(() => {
          socket.close();
          reject(new Error('Event timeout'));
        }, 10000);
        socket.onmessage = (message) => {
          clearTimeout(timeout);
          socket.close();
          resolve(JSON.parse(message.data));
        };
        socket.onerror = () => {
          clearTimeout(timeout);
          reject(new Error('Socket failed'));
        };
      });
    await fetch('/api/v1/admin/jobs', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-Thelxinoe-Client': '1',
      },
      body: JSON.stringify({ key: crypto.randomUUID() }),
    });
    const first = await receive(await ticket(), 0);
    const replay = await receive(await ticket(), first.id - 1);
    return { first, replay };
  });
  expect(events.replay.id).toBe(events.first.id);
  const csrf = await request.post('/api/v1/admin/jobs', {
    headers: { ...headers, Origin: 'https://evil.test' },
    data: { key: 'csrf' },
  });
  expect(csrf.status()).toBe(403);
  const login = await request.post('/api/v1/auth/login', {
    headers,
    data: {
      username: 'admin',
      password: 'test-only long passphrase',
      transport: 'device',
      device_name: 'revocation fixture',
    },
  });
  const deviceToken = (await login.json()).token;
  const sessionResponse = await request.get('/api/v1/auth/sessions', {
    headers: { Authorization: `Bearer ${deviceToken}` },
  });
  const session = (await sessionResponse.json()).current;
  expect(
    (
      await request.delete(`/api/v1/auth/sessions/${session}`, {
        headers: { ...headers, Authorization: `Bearer ${deviceToken}` },
      })
    ).status(),
  ).toBe(200);
  expect(
    (
      await request.get('/api/v1/auth/me', {
        headers: { Authorization: `Bearer ${deviceToken}` },
      })
    ).status(),
  ).toBe(401);
  await page.screenshot({ path: '.local/proxy-settings.png', fullPage: true });
});
