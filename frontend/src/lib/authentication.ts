import { api, ApiError, desktop, serverUrl, type User } from './api';
import { invoke } from '@tauri-apps/api/core';
import { writable, get } from 'svelte/store';
import { captureSession } from './session';

export type AuthOptions = {
  canonical_url: string;
  passkeys: boolean;
  oidc: { label: string; available: boolean } | null;
  server_id: string;
};
export type TotpSetup = {
  secret: string;
  uri: string;
  issuer: string;
  account: string;
  digits: number;
  period: number;
  algorithm: string;
};
/** Readable message for a failed request, including network failures. */
export function errorMessage(caught: unknown) {
  if (caught instanceof ApiError) return caught.message;
  if (caught instanceof TypeError)
    return "Couldn't reach the server. Check your connection and try again.";
  return caught instanceof Error ? caught.message : String(caught);
}
/** Whether passkeys can be used from this page; browsers bind them to the server's hostname. */
export function passkeysHere(options: AuthOptions | null) {
  return (
    !!options?.passkeys &&
    (desktop || options.canonical_url === location.origin)
  );
}
export type AuthMethods = {
  password: boolean;
  totp: boolean;
  passkeys: { id: string; name: string; created_at: number }[];
  oidc: string | null;
  fresh: boolean;
};
export const authNotice = new URLSearchParams(location.search).get(
  'auth_notice',
);
export type LoginResponse = {
  user?: User;
  totp_required?: boolean;
  attempt?: string;
  pending?: boolean;
  verified?: boolean;
};
type JsonDescriptor = Omit<PublicKeyCredentialDescriptor, 'id'> & {
  id: string;
};
type RequestOptions = Omit<
  PublicKeyCredentialRequestOptions,
  'challenge' | 'allowCredentials'
> & { challenge: string; allowCredentials?: JsonDescriptor[] };
type CreationOptions = Omit<
  PublicKeyCredentialCreationOptions,
  'challenge' | 'user' | 'excludeCredentials'
> & {
  challenge: string;
  user: Omit<PublicKeyCredentialUserEntity, 'id'> & { id: string };
  excludeCredentials?: JsonDescriptor[];
};
type PasskeyStart<T> = { attempt: string; options: { publicKey: T } };
function bytes(value: string) {
  return Uint8Array.from(
    atob(value.replace(/-/g, '+').replace(/_/g, '/')),
    (c) => c.charCodeAt(0),
  );
}
function toJson(credential: PublicKeyCredential) {
  const encode = (buffer: ArrayBuffer) =>
    btoa(String.fromCharCode(...new Uint8Array(buffer)))
      .replace(/\+/g, '-')
      .replace(/\//g, '_')
      .replace(/=+$/, '');
  const response = credential.response;
  return {
    id: credential.id,
    rawId: encode(credential.rawId),
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
    response:
      response instanceof AuthenticatorAttestationResponse
        ? {
            attestationObject: encode(response.attestationObject),
            clientDataJSON: encode(response.clientDataJSON),
            transports: response.getTransports(),
          }
        : {
            authenticatorData: encode(
              (response as AuthenticatorAssertionResponse).authenticatorData,
            ),
            clientDataJSON: encode(response.clientDataJSON),
            signature: encode(
              (response as AuthenticatorAssertionResponse).signature,
            ),
            userHandle: (response as AuthenticatorAssertionResponse).userHandle
              ? encode((response as AuthenticatorAssertionResponse).userHandle!)
              : null,
          },
  };
}
/** Signs in with any passkey for this server (the authenticator picks the account), or verifies the signed-in account. */
export async function passkeySignIn(verify = false): Promise<LoginResponse> {
  const start = await api<PasskeyStart<RequestOptions>>(
    '/auth/passkey/start',
    'POST',
    { purpose: verify ? 'verify' : 'login' },
  );
  const options = start.options.publicKey;
  let credential: Credential | null;
  try {
    credential = await navigator.credentials.get({
      publicKey: {
        ...options,
        challenge: bytes(options.challenge),
        allowCredentials: (options.allowCredentials ?? []).map((c) => ({
          ...c,
          id: bytes(c.id),
        })),
      },
    });
  } catch (caught) {
    if (caught instanceof DOMException && caught.name === 'NotAllowedError')
      throw new Error(
        'Passkey sign-in was canceled or timed out, or no passkey for this server is on this device.',
        { cause: caught },
      );
    throw caught;
  }
  if (!credential) throw new Error('Passkey sign-in was canceled');
  return api('/auth/passkey/finish', 'POST', {
    attempt: start.attempt,
    credential: toJson(credential as PublicKeyCredential),
  });
}
export async function registerPasskey(
  name = 'Passkey',
  recoveryToken?: string,
) {
  const start = await api<PasskeyStart<CreationOptions>>(
    '/me/auth/passkeys/register/start',
    'POST',
    { name, recovery_token: recoveryToken },
  );
  const options = start.options.publicKey;
  const credential = await navigator.credentials.create({
    publicKey: {
      ...options,
      challenge: bytes(options.challenge),
      user: { ...options.user, id: bytes(options.user.id) },
      excludeCredentials: (options.excludeCredentials ?? []).map((c) => ({
        ...c,
        id: bytes(c.id),
      })),
    },
  });
  if (!credential) throw new Error('Passkey registration was canceled');
  return api('/me/auth/passkeys/register/finish', 'POST', {
    attempt: start.attempt,
    credential: toJson(credential as PublicKeyCredential),
  });
}
export async function oidcSignIn(purpose = 'login') {
  const { url } = await api<{ url: string }>('/auth/oidc/start', 'POST', {
    purpose,
    return_to: location.pathname + location.search + location.hash,
  });
  location.assign(url);
}
export async function openAuthenticationBrowser(url: string) {
  if (desktop) await invoke('open_auth_browser', { url });
  else location.assign(url);
}
export async function canonicalAccount() {
  const options = await api<AuthOptions>('/auth/methods');
  await openAuthenticationBrowser(`${options.canonical_url}/#settings/account`);
}
type Verification = {
  methods: AuthMethods;
  options: AuthOptions;
  finish: (accepted: boolean) => void;
};
export const verification = writable<Verification | null>(null);
export function cancelVerification() {
  get(verification)?.finish(false);
}
export async function withVerification(action: () => Promise<void>) {
  const owns = captureSession();
  const methods = await api<AuthMethods>('/me/auth');
  if (!owns()) return false;
  if (!methods.fresh) {
    const options = await api<AuthOptions>('/auth/methods');
    if (!owns()) return false;
    const accepted = await new Promise<boolean>((resolve) => {
      cancelVerification();
      const pending: Verification = {
        methods,
        options,
        finish: (accepted) => {
          if (get(verification) !== pending) return;
          verification.set(null);
          resolve(accepted);
        },
      };
      verification.set(pending);
    });
    if (!accepted || !owns()) return false;
  }
  await action();
  return true;
}
/**
 * Signs in (or verifies) this desktop through the system browser. `ready` receives a cancel
 * callback and the pairing code the user must type into the browser to approve.
 */
export async function nativeBrowserSignIn(
  signal: AbortSignal,
  ready: (cancel: () => void, code: string) => void,
  verify = false,
): Promise<LoginResponse> {
  const start = await api<{
    request: string;
    secret: string;
    code: string;
    url: string;
    server_id: string;
  }>('/auth/desktop/start', 'POST', {
    device_name: 'Windows desktop',
    purpose: verify ? 'verify' : 'login',
  });
  const origin = serverUrl();
  ready(() => {
    void api('/auth/desktop/cancel', 'POST', {
      request: start.request,
      secret: start.secret,
    });
  }, start.code);
  try {
    if (signal.aborted) throw new Error('Browser sign-in canceled');
    await openAuthenticationBrowser(start.url);
    const deadline = Date.now() + 300000;
    while (!signal.aborted && Date.now() < deadline) {
      if (serverUrl() !== origin)
        throw new Error('The configured server changed');
      const response = await api<LoginResponse>(
        '/auth/desktop/exchange',
        'POST',
        { request: start.request, secret: start.secret },
      ).catch((caught) => {
        if (caught instanceof ApiError && caught.status === 401)
          throw new Error(
            'The browser request ended: it expired, was canceled or had too many incorrect codes. Try again.',
            { cause: caught },
          );
        throw caught;
      });
      if (signal.aborted) {
        if (response.user && serverUrl() === origin)
          await api('/auth/logout', 'POST').catch(() => {});
        throw new Error('Browser sign-in canceled');
      }
      if (response.user || response.verified) return response;
      await new Promise<void>((resolve) => {
        const timer = setTimeout(done, 1000);
        function done() {
          clearTimeout(timer);
          signal.removeEventListener('abort', done);
          resolve();
        }
        signal.addEventListener('abort', done, { once: true });
      });
    }
    throw new Error(
      signal.aborted ? 'Browser sign-in canceled' : 'Browser sign-in expired',
    );
  } finally {
    await api('/auth/desktop/cancel', 'POST', {
      request: start.request,
      secret: start.secret,
    }).catch(() => {});
  }
}
