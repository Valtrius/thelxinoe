<script lang="ts">
  import { onMount } from 'svelte';
  import { api, desktop } from './api';
  import {
    type AuthMethods,
    type AuthOptions,
    type TotpSetup,
    registerPasskey,
    passkeySignIn,
    oidcSignIn,
    canonicalAccount,
    withVerification,
    authNotice,
    errorMessage,
  } from './authentication';
  import { captureSession } from './session';
  import PasswordSettings from './PasswordSettings.svelte';
  import TotpEnrollment from './TotpEnrollment.svelte';
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
  let enrollment = $state<TotpSetup | null>(null),
    passkeyName = $state('');
  onMount(() => {
    void load().catch((e) => (error = errorMessage(e)));
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
      if (owns()) error = errorMessage(caught);
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
        {#if !methods.totp && !methods.password}<small class="block text-muted"
            >Set a password first. The authenticator app protects password
            sign-in.</small
          >{/if}
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
              enrollment = await api<TotpSetup>('/me/auth/totp/start', 'POST');
            })}>Set up authenticator</Button
        >{/if}
    </div>
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
                await passkeySignIn(true);
                notice = 'Identity verified';
              }, false)}>Verify with passkey</Button
          >{/if}
      </div>
    {:else if options}
      <p class="text-xs text-muted">
        Passkeys aren't available at this address. Browsers only allow them over
        HTTPS with a hostname, or on localhost.
      </p>
    {/if}
    {#if options?.oidc}
      <div class={rowClass}>
        <div class="min-w-0 flex-1">
          <strong>{options.oidc.label}</strong><span class="ml-3 text-muted"
            >{methods.oidc ? 'Linked' : 'Not linked'}</span
          >
          {#if !options.oidc.available}<small class="block text-muted"
              >Identity-provider sign-in needs this server to be reached over
              HTTPS.</small
            >{/if}
        </div>
        {#if methods.oidc}<Button
            variant="secondary"
            disabled={busy || (!methods.password && !methods.passkeys.length)}
            onclick={() => act(() => remove('/me/auth/oidc'))}
            >Unlink identity provider</Button
          >
        {:else}<Button
            variant="secondary"
            disabled={busy || !options.oidc.available}
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
{#if enrollment}<TotpEnrollment
    setup={enrollment}
    onClose={() => {
      enrollment = null;
      void load().catch((e) => (error = errorMessage(e)));
    }}
    onEnabled={() => {
      enrollment = null;
      notice = 'Authenticator app enabled';
      void load().catch((e) => (error = errorMessage(e)));
    }}
  />{/if}
{#if methods}<PasswordSettings
    changed={() => void load()}
    hasPassword={methods.password}
    totp={methods.totp}
    fresh={methods.fresh}
  />{/if}
