<script lang="ts">
  import { onMount } from 'svelte';
  import { KeyRound } from '@lucide/svelte';
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
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';
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

<SettingsCard
  title="Sign-in methods"
  description="Keep at least one method active. The last remaining method cannot be removed."
>
  {#if methods}
    <SettingRow
      label="Password"
      description={methods.password
        ? 'Change it in the card below.'
        : 'Set one in the card below to sign in with a username and password.'}
    >
      {#snippet badge()}
        <StatusChip tone={methods?.password ? 'ok' : 'neutral'}
          >{methods?.password ? 'Enabled' : 'Not set'}</StatusChip
        >
      {/snippet}
      {#if methods.password}<Button
          variant="ghost"
          size="form"
          disabled={busy || !(methods.passkeys.length || methods.oidc)}
          onclick={() => act(() => remove('/me/auth/password'))}
          >Remove password</Button
        >{/if}
    </SettingRow>
    <SettingRow
      label="Authenticator app"
      description={!methods.totp && !methods.password
        ? 'Set a password first. The authenticator app protects password sign-in.'
        : 'One-time codes that protect password sign-in.'}
    >
      {#snippet badge()}
        <StatusChip tone={methods?.totp ? 'ok' : 'neutral'}
          >{methods?.totp ? 'Enabled' : 'Not enabled'}</StatusChip
        >
      {/snippet}
      {#if methods.totp}<Button
          variant="ghost"
          size="form"
          disabled={busy}
          onclick={() => act(() => remove('/me/auth/totp'))}
          >Remove authenticator</Button
        >
      {:else}<Button
          variant="secondary"
          size="form"
          disabled={busy || !methods.password}
          onclick={() =>
            act(async () => {
              enrollment = await api<TotpSetup>('/me/auth/totp/start', 'POST');
            })}>Set up authenticator</Button
        >{/if}
    </SettingRow>
    <SettingRow
      label="Passkeys"
      description={options && !options.passkeys
        ? "Passkeys aren't available at this address. Browsers only allow them over HTTPS with a hostname, or on localhost."
        : 'Sign in with your device’s fingerprint, face or PIN.'}
    >
      {#snippet badge()}
        <StatusChip
          tone={methods?.passkeys.length ? 'info' : 'neutral'}
          dot={false}
          >{methods?.passkeys.length
            ? `${methods.passkeys.length} ${methods.passkeys.length === 1 ? 'passkey' : 'passkeys'}`
            : 'None'}</StatusChip
        >
      {/snippet}
      {#if options?.passkeys && !atCanonical}<Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() => canonicalAccount()}>Manage passkeys in browser</Button
        >{/if}
    </SettingRow>
    {#each methods.passkeys as passkey (passkey.id)}
      <SettingRow sub label={passkey.name}>
        {#snippet lead()}
          <KeyRound size={15} class="shrink-0 text-muted" aria-hidden="true" />
        {/snippet}
        <Button
          variant="ghost"
          size="form"
          aria-label={`Remove passkey ${passkey.name}`}
          disabled={busy ||
            (!methods?.password &&
              !methods?.oidc &&
              methods?.passkeys.length === 1)}
          onclick={() => act(() => remove(`/me/auth/passkeys/${passkey.id}`))}
          >Remove</Button
        >
      </SettingRow>
    {/each}
    {#if options?.passkeys && atCanonical}
      <SettingRow
        sub
        label="Add a passkey"
        description="Name it after the device or security key you will use."
      >
        <input
          class={twMerge(formControlClass, 'w-44 compact:w-full')}
          aria-label="Passkey name"
          bind:value={passkeyName}
          placeholder="Passkey"
          maxlength="100"
          disabled={busy}
        />
        <Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() =>
            act(async () => {
              await registerPasskey(passkeyName || 'Passkey');
              passkeyName = '';
              notice = 'Passkey added';
            })}>Add passkey</Button
        >
        {#if methods.passkeys.length}<Button
            variant="ghost"
            size="form"
            disabled={busy}
            onclick={() =>
              act(async () => {
                await passkeySignIn(true);
                notice = 'Identity verified';
              }, false)}>Verify with passkey</Button
          >{/if}
      </SettingRow>
    {/if}
    {#if options?.oidc}
      <SettingRow
        label={options.oidc.label}
        description={options.oidc.available
          ? 'Single sign-on through your identity provider.'
          : 'Identity-provider sign-in needs this server to be reached over HTTPS.'}
      >
        {#snippet badge()}
          <StatusChip tone={methods?.oidc ? 'ok' : 'neutral'}
            >{methods?.oidc ? 'Linked' : 'Not linked'}</StatusChip
          >
        {/snippet}
        {#if methods.oidc}<Button
            variant="ghost"
            size="form"
            disabled={busy || (!methods.password && !methods.passkeys.length)}
            onclick={() => act(() => remove('/me/auth/oidc'))}
            >Unlink identity provider</Button
          >
        {:else}<Button
            variant="secondary"
            size="form"
            disabled={busy || !options.oidc.available}
            onclick={() =>
              !atCanonical ? canonicalAccount() : act(() => oidcSignIn('link'))}
            >Link identity provider</Button
          >{/if}
      </SettingRow>
    {/if}
  {:else if !error}
    <div class={cardBlockClass}>
      <ContentSkeleton label="Loading sign-in methods" count={3} />
    </div>
  {/if}
  {#if error || notice}
    <div class={cardBlockClass}>
      {#if error}<Notice role="alert" variant="error" class="mb-0"
          >{error}</Notice
        >{/if}
      {#if notice}<p role="status" class="m-0 text-xs text-muted">
          {notice}
        </p>{/if}
    </div>
  {/if}
</SettingsCard>
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
