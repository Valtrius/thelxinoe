import { expect, test } from '@playwright/test';
import { installUiFixture } from './helpers/ui-fixture';

test('sign-in rechecks setup and requires confirmation before creating an account', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  let created = false;
  await page.route('**/api/v1/setup', async (route) => {
    if (route.request().method() === 'POST') {
      created = true;
      return route.fulfill({
        json: {
          user: {
            id: 'layout-fixture',
            username: 'admin',
            role: 'admin',
            timezone: 'UTC',
          },
        },
      });
    }
    return route.fulfill({ json: { setup_required: true } });
  });
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByLabel('Confirm password', { exact: true }),
  ).toBeVisible();
  expect(created).toBe(false);
  expect(fixture.writes.some((write) => write.path === '/auth/login')).toBe(
    false,
  );
  await page
    .getByLabel('Confirm password', { exact: true })
    .fill('local test passphrase');
  await page
    .getByRole('button', { name: 'Create your server', exact: true })
    .click();
  await expect(
    page.getByRole('button', { name: 'Sign out', exact: true }),
  ).toBeVisible();
  expect(created).toBe(true);
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('account-created', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('sign-in checks compatibility before sending credentials', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.goto('/');
  await page.getByLabel('Username', { exact: true }).fill('admin');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  await page.route('**/api/v1/health', (route) =>
    route.fulfill({ json: { api_version: 99, api_min: 99, api_max: 99 } }),
  );
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(
    page.getByRole('heading', { name: 'Update required', exact: true }),
  ).toBeVisible();
  expect(fixture.writes.some((write) => write.path === '/auth/login')).toBe(
    false,
  );
  expect(fixture.errors).toEqual([]);
  await testInfo.attach('incompatible', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
});

test('authenticator enrollment shows a theme-independent QR code, copies the key and explains failures', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], {
    origin: new URL(testInfo.project.use.baseURL!).origin,
  });
  const secret = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';
  const confirmations: unknown[] = [];
  await page.route('**/api/v1/me/auth/totp/start', (route) =>
    route.fulfill({
      json: {
        secret,
        uri: `otpauth://totp/Thelxinoe:viewer%40127.0.0.1?secret=${secret}&issuer=Thelxinoe`,
        issuer: 'Thelxinoe',
        account: 'viewer@127.0.0.1',
        digits: 6,
        period: 30,
        algorithm: 'SHA1',
      },
    }),
  );
  await page.route('**/api/v1/me/auth/totp/confirm', (route) => {
    confirmations.push(route.request().postDataJSON());
    return confirmations.length === 1
      ? route.fulfill({
          status: 400,
          json: {
            error: {
              code: 'totp_incorrect',
              message: 'That code is incorrect or expired.',
            },
          },
        })
      : route.fulfill({ json: { saved: true } });
  });
  await page.goto('/');
  await page
    .getByRole('button', { name: 'Set up authenticator', exact: true })
    .click();
  const dialog = page.getByRole('dialog', {
    name: 'Set up authenticator app',
  });
  const qr = dialog.getByRole('img', {
    name: 'Authenticator QR code for viewer@127.0.0.1',
  });
  await expect(qr).toBeVisible();
  expect((await qr.boundingBox())!.width).toBeGreaterThanOrEqual(160);
  for (const theme of ['light', 'dark']) {
    await page.evaluate((value) => {
      document.documentElement.dataset.theme = value;
    }, theme);
    expect(
      await qr.evaluate((svg) => ({
        field: getComputedStyle(svg.querySelector('rect')!).fill,
        modules: getComputedStyle(svg.querySelector('path')!).fill,
        filter: getComputedStyle(svg).filter,
      })),
    ).toEqual({
      field: 'rgb(255, 255, 255)',
      modules: 'rgb(0, 0, 0)',
      filter: 'none',
    });
    await testInfo.attach(`authenticator-enrollment-${theme}`, {
      body: await page.screenshot(),
      contentType: 'image/png',
    });
  }
  await expect(dialog.getByLabel('Setup key', { exact: true })).toHaveValue(
    'JBSW Y3DP EHPK 3PXP JBSW Y3DP EHPK 3PXP',
  );
  await dialog
    .getByRole('button', { name: 'Copy setup key', exact: true })
    .click();
  await expect(
    dialog.getByText('Setup key copied', { exact: true }),
  ).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(
    secret,
  );
  const code = dialog.getByLabel('Authentication code', { exact: true });
  const enable = dialog.getByRole('button', {
    name: 'Enable authenticator',
    exact: true,
  });
  await code.fill('12');
  await enable.click();
  await expect(dialog.getByRole('alert')).toContainText(
    'Enter the 6-digit code',
  );
  expect(confirmations).toEqual([]);
  await code.fill('123456');
  await enable.click();
  await expect(dialog.getByRole('alert')).toContainText('set automatically');
  await expect(code).toHaveValue('');
  await code.fill('654 321');
  await enable.click();
  await expect(
    dialog.getByText('Authenticator app enabled', { exact: true }),
  ).toBeVisible();
  expect(confirmations).toEqual([{ code: '123456' }, { code: '654321' }]);
  await dialog.getByRole('button', { name: 'Done', exact: true }).click();
  await expect(dialog).not.toBeVisible();
  await expect(
    page.getByText('Authenticator app enabled', { exact: true }),
  ).toBeVisible();
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('desktop approval requires the desktop code and flags requests from another network', async ({
  page,
}, testInfo) => {
  const fixture = await installUiFixture(page);
  const approvals: unknown[] = [];
  await page.route('**/api/v1/auth/desktop/info?**', (route) =>
    route.fulfill({
      json: {
        name: 'Windows desktop',
        verifying: null,
        requested_from: '203.0.113.9',
        requested_at: Math.floor(Date.now() / 1000),
        same_network: false,
        server_id: 'ui-fixture',
      },
    }),
  );
  await page.route('**/api/v1/auth/desktop/approve', (route) => {
    approvals.push(route.request().postDataJSON());
    return approvals.length === 1
      ? route.fulfill({
          status: 400,
          json: {
            error: {
              code: 'desktop_code_mismatch',
              message:
                "That code doesn't match the one shown in the desktop app.",
            },
          },
        })
      : route.fulfill({ json: { approved: true } });
  });
  await page.goto('/?desktop=fixture-request');
  await expect(
    page.getByRole('heading', { name: 'Connect desktop', exact: true }),
  ).toBeVisible();
  await expect(page.getByText(/different network address/)).toBeVisible();
  await expect(page.getByText(/from 203\.0\.113\.9/)).toBeVisible();
  const code = page.getByLabel('Code shown in the desktop app', {
    exact: true,
  });
  const approve = page.getByRole('button', {
    name: 'Approve desktop',
    exact: true,
  });
  await approve.click();
  await expect(page.getByRole('alert')).toContainText('8-character code');
  expect(approvals).toEqual([]);
  await code.fill('WRNG-CODE');
  await approve.click();
  await expect(page.getByRole('alert')).toContainText("doesn't match");
  await testInfo.attach('desktop-approval', {
    body: await page.screenshot(),
    contentType: 'image/png',
  });
  await code.fill('abcd-efgh');
  await approve.click();
  await expect(page.getByText(/^Desktop approved/)).toBeVisible();
  expect(approvals).toEqual([
    { request: 'fixture-request', code: 'WRNG-CODE' },
    { request: 'fixture-request', code: 'abcd-efgh' },
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});

test('an expired authentication-code step returns to password sign-in with an explanation', async ({
  page,
}) => {
  const fixture = await installUiFixture(page, { signedIn: false });
  await page.route('**/api/v1/auth/methods', (route) =>
    route.fulfill({
      json: {
        canonical_url: new URL(route.request().url()).origin,
        passkeys: true,
        oidc: null,
        server_id: 'ui-fixture',
      },
    }),
  );
  await page.route('**/api/v1/auth/login', (route) =>
    route.fulfill({ json: { totp_required: true, attempt: 'fixture' } }),
  );
  await page.route('**/api/v1/auth/totp', (route) =>
    route.fulfill({
      status: 401,
      json: {
        error: {
          code: 'sign_in_expired',
          message:
            'Too many incorrect codes. Sign in again with your password.',
        },
      },
    }),
  );
  await page.goto('/');
  // Usernameless passkeys need no username first.
  await expect(
    page.getByRole('button', { name: 'Sign in with passkey', exact: true }),
  ).toBeEnabled();
  await page.getByLabel('Username', { exact: true }).fill('viewer');
  await page
    .getByLabel('Password', { exact: true })
    .fill('local test passphrase');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.getByLabel('Authentication code', { exact: true }).fill('123456');
  await page.getByRole('button', { name: 'Verify code', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText(
    'Too many incorrect codes',
  );
  await expect(page.getByLabel('Password', { exact: true })).toHaveValue('');
  await expect(
    page.getByLabel('Authentication code', { exact: true }),
  ).toHaveCount(0);
  expect(fixture.errors).toEqual([]);
});

test('a password change can also revoke client passwords', async ({ page }) => {
  const fixture = await installUiFixture(page);
  let clients = [{ id: 'tv', name: 'Living room TV' }];
  const changes: unknown[] = [];
  await page.route('**/api/v1/me/auth/client-passwords', (route) =>
    route.fulfill({ json: { items: clients } }),
  );
  await page.route('**/api/v1/me/password', (route) => {
    const body = route.request().postDataJSON();
    changes.push(body);
    if (body.revoke_client_passwords) clients = [];
    return route.fulfill({ json: { saved: true } });
  });
  await page.goto('/');
  const revoke = page.getByLabel('Also revoke client passwords', {
    exact: true,
  });
  await expect(revoke).not.toBeChecked();
  await expect(page.getByText(/signed in with Living room TV/)).toBeVisible();
  const change = async (password: string) => {
    await page.getByLabel('New password', { exact: true }).fill(password);
    await page
      .getByLabel('Confirm new password', { exact: true })
      .fill(password);
    await page
      .getByRole('button', { name: 'Change password', exact: true })
      .click();
  };
  await change('first replacement passphrase');
  await expect(page.getByRole('status')).toContainText(
    'Media apps using a client password stay signed in',
  );
  await revoke.check();
  await change('second replacement passphrase');
  await expect(page.getByRole('status')).toContainText(
    'client passwords revoked',
  );
  await expect(revoke).toHaveCount(0);
  expect(changes).toEqual([
    {
      current_password: '',
      new_password: 'first replacement passphrase',
      revoke_client_passwords: false,
    },
    {
      current_password: '',
      new_password: 'second replacement passphrase',
      revoke_client_passwords: true,
    },
  ]);
  expect(fixture.errors).toEqual([]);
  expect(fixture.unexpected).toEqual([]);
});
