import { expect } from '@playwright/test';
import { createHmac } from 'node:crypto';
import { join } from 'node:path';

export async function authenticationScenario({
  native,
  lab,
  credentials,
  scenario,
  output,
}) {
  await scenario(
    'Native browser exchange writes only to the isolated OS keyring and survives reload',
    async () => {
      const headers = {
        'X-Thelxinoe-Client': '1',
        'Content-Type': 'application/json',
      };
      const invoke = (path, method = 'GET', body = null) =>
        native.invoke('backend_request', { path, method, body });
      const setup = await invoke('/me/auth/totp/start', 'POST', {});
      expect(setup.status).toBe(200);
      const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
      const bits = [...setup.body.secret]
        .map((c) => alphabet.indexOf(c).toString(2).padStart(5, '0'))
        .join('');
      const secret = Buffer.from(
        bits.match(/.{8}/g).map((b) => parseInt(b, 2)),
      );
      const counter = Buffer.alloc(8);
      counter.writeBigUInt64BE(BigInt(Math.floor(Date.now() / 30000)));
      const hash = createHmac('sha1', secret).update(counter).digest();
      const code = (
        (hash.readUInt32BE(hash[hash.length - 1] & 15) & 0x7fffffff) %
        1000000
      )
        .toString()
        .padStart(6, '0');
      expect(
        (await invoke('/me/auth/totp/confirm', 'POST', { code })).status,
      ).toBe(200);
      const response = await fetch(`${lab.baseUrl}/api/v1/auth/login`, {
        method: 'POST',
        headers,
        body: JSON.stringify(credentials),
      });
      const challenge = await response.json();
      const verifiedStep = Math.floor(Date.now() / 30000) + 1;
      counter.writeBigUInt64BE(BigInt(verifiedStep));
      const nextHash = createHmac('sha1', secret).update(counter).digest();
      const nextCode = (
        (nextHash.readUInt32BE(nextHash[nextHash.length - 1] & 15) &
          0x7fffffff) %
        1000000
      )
        .toString()
        .padStart(6, '0');
      const verified = await fetch(`${lab.baseUrl}/api/v1/auth/totp`, {
        method: 'POST',
        headers,
        body: JSON.stringify({ attempt: challenge.attempt, code: nextCode }),
      });
      expect(verified.ok).toBe(true);
      const authHeaders = {
        ...headers,
        Cookie: verified.headers.get('set-cookie').split(';')[0],
      };
      expect((await invoke('/auth/logout', 'POST', {})).status).toBe(200);
      const pending = await invoke('/auth/login', 'POST', credentials);
      expect(pending.body.totp_required).toBe(true);
      expect(pending.body.token).toBeUndefined();
      expect((await invoke('/auth/me')).status).toBe(401);
      const started = (
        await invoke('/auth/desktop/start', 'POST', {
          device_name: 'Native authentication E2E',
        })
      ).body;
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/desktop/approve`, {
            method: 'POST',
            headers: authHeaders,
            body: JSON.stringify({ request: started.request }),
          })
        ).ok,
      ).toBe(true);
      const completed = await invoke('/auth/desktop/exchange', 'POST', {
        request: started.request,
        secret: started.secret,
      });
      expect(completed.status).toBe(200);
      expect(completed.body.token).toBeUndefined();
      expect(completed.body.user.username).toBe(credentials.username);
      expect(
        (
          await invoke('/auth/desktop/exchange', 'POST', {
            request: started.request,
            secret: started.secret,
          })
        ).status,
      ).toBe(401);
      await native.page.reload();
      await expect(
        native.page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible();
      const state = await native.page.evaluate(() => ({
        cookie: document.cookie,
        local: Object.keys(localStorage),
        session: Object.keys(sessionStorage),
      }));
      expect(state.cookie).not.toContain('thelxinoe_session');
      expect(
        [...state.local, ...state.session].some((key) =>
          /token|session|password|remember/i.test(key),
        ),
      ).toBe(false);
      await native.page.screenshot({
        path: join(output, 'native-authentication.png'),
      });
      // A globally consumed TOTP step cannot be reused on another transport.
      while (Math.floor(Date.now() / 30000) < verifiedStep) {
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
      await invoke('/auth/logout', 'POST', {});
      const rememberChallenge = await invoke(
        '/auth/login',
        'POST',
        credentials,
      );
      counter.writeBigUInt64BE(
        BigInt(Math.max(verifiedStep + 1, Math.floor(Date.now() / 30000))),
      );
      const rememberHash = createHmac('sha1', secret).update(counter).digest();
      const rememberCode = (
        (rememberHash.readUInt32BE(rememberHash[rememberHash.length - 1] & 15) &
          0x7fffffff) %
        1000000
      )
        .toString()
        .padStart(6, '0');
      const remembered = await invoke('/auth/totp', 'POST', {
        attempt: rememberChallenge.body.attempt,
        code: rememberCode,
        remember_device: true,
      });
      expect(remembered.status).toBe(200);
      expect(remembered.body.token).toBeUndefined();
      expect(remembered.body.remember_token).toBeUndefined();
      await native.page.reload();
      await expect(
        native.page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible();
      const rememberedSessions = await invoke('/auth/sessions');
      expect(
        rememberedSessions.body.items.find(
          (s) => s.id === rememberedSessions.body.current,
        ).remembered,
      ).toBe(true);
      await invoke('/auth/logout', 'POST', {});
      const resumed = await invoke('/auth/login', 'POST', credentials);
      expect(resumed.body.user.username).toBe(credentials.username);
      expect(resumed.body.totp_required).toBeUndefined();
      expect((await invoke('/me/auth')).body.fresh).toBe(false);
      const proofRequest = (
        await invoke('/auth/desktop/start', 'POST', {
          purpose: 'verify',
          device_name: 'Existing native session',
        })
      ).body;
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/desktop/approve`, {
            method: 'POST',
            headers: authHeaders,
            body: JSON.stringify({ request: proofRequest.request }),
          })
        ).status,
      ).toBe(200);
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/desktop/exchange`, {
            method: 'POST',
            headers: authHeaders,
            body: JSON.stringify({
              request: proofRequest.request,
              secret: proofRequest.secret,
            }),
          })
        ).status,
      ).toBe(401);
      const proofExchange = await invoke('/auth/desktop/exchange', 'POST', {
        request: proofRequest.request,
        secret: proofRequest.secret,
      });
      expect(proofExchange.body.verified).toBe(true);
      expect(proofExchange.body.token).toBeUndefined();
      expect((await invoke('/me/auth')).body.fresh).toBe(true);
      await native.page.reload();
      await expect(
        native.page.getByRole('link', { name: 'Settings', exact: true }),
      ).toBeVisible();
      await native.page.screenshot({
        path: join(output, 'native-remembered-verification.png'),
      });
      // Restore the account for the existing update scenarios using the genuinely verified session.
      expect((await invoke('/me/auth/totp', 'DELETE')).status).toBe(200);
      const username = `recovery${Date.now()}`;
      const created = await invoke('/users', 'POST', {
        username,
        password: 'native recovery fixture passphrase',
        role: 'user',
      });
      expect(created.status).toBe(200);
      await native.page
        .getByRole('link', { name: 'Settings', exact: true })
        .click();
      await native.page
        .getByRole('navigation', { name: 'Settings navigation' })
        .getByRole('link', { name: 'People', exact: true })
        .click();
      await native.page
        .getByRole('button', { name: new RegExp(username) })
        .click();
      await native.page
        .getByRole('button', { name: 'Create recovery link', exact: true })
        .click();
      const recovery = native.page.getByLabel('Recovery link', { exact: true });
      await expect(recovery).toHaveValue(/^https?:\/\/.+\/#recovery=/);
      const recoveryUrl = await recovery.inputValue();
      expect(new URL(recoveryUrl).origin).toBe(new URL(lab.baseUrl).origin);
      const token = new URL(recoveryUrl).hash.slice('#recovery='.length);
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/recovery/info`, {
            method: 'POST',
            headers,
            body: JSON.stringify({ token }),
          })
        ).status,
      ).toBe(200);
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/recovery/enroll`, {
            method: 'POST',
            headers,
            body: JSON.stringify({
              token,
              password: 'native recovered fixture passphrase',
            }),
          })
        ).status,
      ).toBe(200);
      expect(
        (
          await fetch(`${lab.baseUrl}/api/v1/auth/login`, {
            method: 'POST',
            headers,
            body: JSON.stringify({
              username,
              password: 'native recovered fixture passphrase',
            }),
          })
        ).status,
      ).toBe(200);
      await native.page.screenshot({
        path: join(output, 'native-recovery-origin.png'),
      });
      expect((await invoke(`/users/${created.body.id}`, 'DELETE')).status).toBe(
        200,
      );
    },
  );
}
