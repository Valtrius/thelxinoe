<script lang="ts">
  import { onMount } from 'svelte';
  import { api, desktop } from './api';
  import {
    type AuthMethods,
    type AuthOptions,
    registerPasskey,
    passkeySignIn,
    oidcSignIn,
    canonicalAccount,
    withVerification,
    authNotice,
  } from './authentication';
  import { captureSession } from './session';
  import PasswordSettings from './PasswordSettings.svelte';
  import Panel from './ui/Panel.svelte';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass, rowClass } from './ui/styles';
  let methods = $state<AuthMethods | null>(null),
    options = $state<AuthOptions | null>(null);
  let busy = $state(false),
    error = $state(''),
    notice = $state('');
  let enrollment = $state<{ secret: string; uri: string } | null>(null),
    code = $state(''),
    passkeyName = $state('');
  onMount(() => {
    void load().catch((e) => (error = String(e)));
    if (authNotice === 'oidc-linked') notice = 'OIDC account linked';
    else if (authNotice === 'verified') notice = 'Identity verified';
  });
  async function load() {
    const owns = captureSession();
    const [value, available] = await Promise.all([
      api<AuthMethods>('/me/auth'),
      api<AuthOptions>('/auth/methods'),
    ]);
    if (owns()) {
      methods = value;
      options = available;
    }
  }
  const atCanonical = $derived(
    !desktop && options?.canonical_url === location.origin,
  );
  async function act(action: () => Promise<void>, fresh = true) {
    const owns = captureSession();
    busy = true;
    error = '';
    notice = '';
    try {
      if (fresh) await withVerification(action);
      else await action();
      if (owns()) await load();
    } catch (caught) {
      if (owns()) error = String(caught);
    } finally {
      if (owns()) busy = false;
    }
  }
  async function remove(path: string) {
    await api(path, 'DELETE');
    notice = 'Sign-in method removed';
  }
</script>

<Panel>
  <h2>Sign-in methods</h2>
  {#if methods}
    <div class={rowClass}>
      <div class="min-w-0 flex-1">
        <strong>Password</strong><span class="ml-3 text-muted"
          >{methods.password ? 'Enabled' : 'Not set'}</span
        >
      </div>
      {#if methods.password}<Button
          variant="secondary"
          disabled={busy || !(methods.passkeys.length || methods.oidc)}
          onclick={() => act(() => remove('/me/auth/password'))}
          >Remove password</Button
        >{/if}
    </div>
    <div class={rowClass}>
      <div class="min-w-0 flex-1">
        <strong>Authenticator app</strong><span class="ml-3 text-muted"
          >{methods.totp ? 'Enabled for password sign-in' : 'Not enabled'}</span
        >
      </div>
      {#if methods.totp}<Button
          variant="secondary"
          disabled={busy}
          onclick={() => act(() => remove('/me/auth/totp'))}
          >Remove authenticator</Button
        >
      {:else}<Button
          variant="secondary"
          disabled={busy || !methods.password}
          onclick={() =>
            act(async () => {
              enrollment = await api('/me/auth/totp/start', 'POST');
            })}>Set up authenticator</Button
        >{/if}
    </div>
    {#if enrollment}
      <form
        class="max-w-120 border-b border-line py-4"
        onsubmit={(e) => {
          e.preventDefault();
          void act(async () => {
            await api('/me/auth/totp/confirm', 'POST', { code });
            enrollment = null;
            code = '';
            notice = 'Authenticator enabled';
          });
        }}
      >
        <FormField
          >Setup key<input
            class={formControlClass}
            readonly
            value={enrollment.secret}
          /></FormField
        >
        <a class="text-accent" href={enrollment.uri}>Open authenticator app</a>
        <FormField
          >Authentication code<input
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
        <div class="flex gap-2">
          <Button type="submit" disabled={busy}>Enable authenticator</Button
          ><Button
            variant="secondary"
            disabled={busy}
            onclick={() => {
              enrollment = null;
              code = '';
            }}>Cancel</Button
          >
        </div>
      </form>
    {/if}
    {#each methods.passkeys as passkey (passkey.id)}
      <div class={rowClass}>
        <strong class="min-w-0 flex-1 truncate">{passkey.name}</strong><Button
          variant="secondary"
          aria-label={`Remove passkey ${passkey.name}`}
          disabled={busy ||
            (!methods.password &&
              !methods.oidc &&
              methods.passkeys.length === 1)}
          onclick={() => act(() => remove(`/me/auth/passkeys/${passkey.id}`))}
          >Remove</Button
        >
      </div>
    {/each}
    {#if options?.passkeys}
      {#if atCanonical}<div class="max-w-120 pt-3">
          <FormField
            >Passkey name<input
              class={formControlClass}
              bind:value={passkeyName}
              placeholder="Passkey"
              maxlength="100"
              disabled={busy}
            /></FormField
          >
        </div>{/if}
      <div class="flex flex-wrap gap-2 py-3">
        <Button
          variant="secondary"
          disabled={busy}
          onclick={() =>
            !atCanonical
              ? canonicalAccount()
              : act(async () => {
                  await registerPasskey(passkeyName || 'Passkey');
                  passkeyName = '';
                  notice = 'Passkey added';
                })}
          >{atCanonical ? 'Add passkey' : 'Manage passkeys in browser'}</Button
        >
        {#if atCanonical && methods.passkeys.length}<Button
            variant="secondary"
            disabled={busy}
            onclick={() =>
              act(async () => {
                await passkeySignIn('', true);
                notice = 'Identity verified';
              }, false)}>Verify with passkey</Button
          >{/if}
      </div>
    {/if}
    {#if options?.oidc}
      <div class={rowClass}>
        <div class="min-w-0 flex-1">
          <strong>{options.oidc.label}</strong><span class="ml-3 text-muted"
            >{methods.oidc ? 'Linked' : 'Not linked'}</span
          >
        </div>
        {#if methods.oidc}<Button
            variant="secondary"
            disabled={busy || (!methods.password && !methods.passkeys.length)}
            onclick={() => act(() => remove('/me/auth/oidc'))}
            >Unlink identity provider</Button
          >
        {:else}<Button
            variant="secondary"
            disabled={busy}
            onclick={() =>
              !atCanonical ? canonicalAccount() : act(() => oidcSignIn('link'))}
            >Link identity provider</Button
          >{/if}
      </div>
    {/if}
  {/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
</Panel>
{#if methods}<PasswordSettings
    changed={() => void load()}
    hasPassword={methods.password}
    totp={methods.totp}
    fresh={methods.fresh}
  />{/if}
