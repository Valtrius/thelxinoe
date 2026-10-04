<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api, desktop, type User } from './api';
  import {
    type AuthOptions,
    type LoginResponse,
    passkeySignIn,
    oidcSignIn,
    nativeBrowserSignIn,
    openAuthenticationBrowser,
  } from './authentication';
  import FormField from './ui/FormField.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let {
    setup,
    serverAddress = $bindable(''),
    options,
    prepare,
    authenticated,
    initialError = '',
  } = $props<{
    setup: boolean;
    serverAddress: string;
    options: AuthOptions | null;
    prepare: (server: string) => Promise<boolean>;
    authenticated: (user: User) => Promise<void>;
    initialError?: string;
  }>();
  let username = $state(''),
    password = $state(''),
    confirmation = $state('');
  let attempt = $state(''),
    code = $state(''),
    remember = $state(false),
    busy = $state(false),
    error = $state(untrack(() => initialError));
  let browser = $state<AbortController | null>(null),
    cancelBrowser: (() => void) | undefined;
  onDestroy(() => {
    browser?.abort();
    cancelBrowser?.();
  });
  async function act(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      busy = false;
    }
  }
  async function accept(response: LoginResponse) {
    password = '';
    confirmation = '';
    if (response.totp_required && response.attempt) {
      attempt = response.attempt;
      code = '';
      remember = false;
    } else if (response.user) {
      attempt = '';
      await authenticated(response.user);
    } else throw new Error('The server did not complete sign-in');
  }
  async function passwordSignIn() {
    await act(async () => {
      if (!(await prepare(serverAddress))) return;
      if (setup && password !== confirmation)
        throw new Error('Passwords do not match.');
      await accept(
        await api<LoginResponse>(setup ? '/setup' : '/auth/login', 'POST', {
          username,
          password,
        }),
      );
    });
  }
  async function browserSignIn() {
    await act(async () => {
      if (!(await prepare(serverAddress))) return;
      browser = new AbortController();
      try {
        await accept(
          await nativeBrowserSignIn(
            browser.signal,
            (cancel) => (cancelBrowser = cancel),
          ),
        );
      } finally {
        browser = null;
        cancelBrowser = undefined;
      }
    });
  }
</script>

<img
  class="block size-10.5 object-contain"
  src="/icon.svg"
  alt="Thelxinoe"
  width="42"
  height="42"
/>
<h1>
  {attempt
    ? 'Authentication code'
    : setup
      ? 'Welcome to Thelxinoe'
      : 'Welcome back'}
</h1>
{#if setup}<p class="text-muted">
    Create the administrator account for your media server.
  </p>{/if}
{#if attempt}
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void act(async () =>
        accept(
          await api<LoginResponse>('/auth/totp', 'POST', {
            attempt,
            code,
            remember_device: remember,
          }),
        ),
      );
    }}
  >
    <FormField
      >Authentication code<input
        {@attach (element) => element.focus()}
        class={formControlClass}
        bind:value={code}
        inputmode="numeric"
        autocomplete="one-time-code"
        pattern={'[0-9]{6}'}
        maxlength="6"
        required
        disabled={busy}
      /></FormField
    >
    <label class="my-4 flex items-center gap-2 text-sm"
      ><input type="checkbox" bind:checked={remember} disabled={busy} />Remember
      this device</label
    >
    <Button size="form" type="submit" class="w-full" disabled={busy}
      >Verify code</Button
    >
    <Button
      size="form"
      variant="secondary"
      class="mt-3 w-full"
      disabled={busy}
      onclick={() => {
        attempt = '';
        code = '';
        remember = false;
      }}>Back to sign in</Button
    >
  </form>
{:else}
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void passwordSignIn();
    }}
  >
    {#if desktop}<FormField
        >Server address<input
          class={formControlClass}
          bind:value={serverAddress}
          required
          disabled={busy}
          placeholder="https://media.example.com"
        /></FormField
      >{/if}
    <FormField
      >Username<input
        {@attach (element) => element.focus()}
        class={formControlClass}
        bind:value={username}
        required
        autocomplete="username"
        disabled={busy}
      /></FormField
    >
    <FormField
      >Password<input
        class={formControlClass}
        bind:value={password}
        type="password"
        required
        minlength={setup ? 8 : 1}
        autocomplete={setup ? 'new-password' : 'current-password'}
        disabled={busy}
      /></FormField
    >
    {#if setup}<FormField
        >Confirm password<input
          class={formControlClass}
          bind:value={confirmation}
          type="password"
          required
          minlength="8"
          autocomplete="new-password"
          disabled={busy}
        /></FormField
      >
      <p class="text-[11px] text-muted">
        Use at least 8 characters. You can create other users after setup.
      </p>{/if}
    <Button size="form" type="submit" class="w-full" disabled={busy}
      >{busy ? 'Connecting…' : setup ? 'Create your server' : 'Sign in'}</Button
    >
  </form>
  {#if !setup}
    <div class="mt-4 flex flex-col gap-2">
      {#if desktop}<Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() => void browserSignIn()}>Sign in with browser</Button
        >
      {:else}
        {#if options?.passkeys && options.canonical_url === location.origin}<Button
            variant="secondary"
            size="form"
            disabled={busy || !username.trim()}
            onclick={() =>
              act(async () => accept(await passkeySignIn(username)))}
            >Sign in with passkey</Button
          >{/if}
        {#if options?.oidc}<Button
            variant="secondary"
            size="form"
            disabled={busy}
            onclick={() =>
              act(async () => {
                if (options && options.canonical_url !== location.origin)
                  await openAuthenticationBrowser(
                    options.canonical_url + location.pathname + location.search,
                  );
                else await oidcSignIn();
              })}>{options.oidc.label}</Button
          >{/if}
        {#if options?.passkeys && options.canonical_url !== location.origin}<Button
            variant="secondary"
            size="form"
            disabled={busy}
            onclick={() =>
              options &&
              openAuthenticationBrowser(
                options.canonical_url + location.pathname + location.search,
              )}>Sign in at authentication hostname</Button
          >{/if}
      {/if}
    </div>
  {/if}
{/if}
{#if browser}<p role="status">Approve this desktop in your browser.</p>
  <Button
    variant="secondary"
    onclick={() => {
      browser?.abort();
      cancelBrowser?.();
    }}>Cancel browser sign-in</Button
  >{/if}
{#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
