<script lang="ts">
  import {
    verification,
    passkeySignIn,
    passkeysHere,
    oidcSignIn,
    nativeBrowserSignIn,
    errorMessage,
  } from './authentication';
  import { api, desktop } from './api';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let proof = $state(''),
    busy = $state(false),
    error = $state(''),
    pairingCode = $state('');
  let controller = $state<AbortController | null>(null);
  $effect(() => {
    if (!$verification) {
      controller?.abort();
      proof = '';
      error = '';
    }
  });
  const methods = $derived($verification?.methods);
  const options = $derived($verification?.options ?? null);
  // Desktop verification goes through the system browser, which can use any method.
  const passkey = $derived(
    !!methods?.passkeys.length && (desktop || passkeysHere(options)),
  );
  const passkeyElsewhere = $derived(!!methods?.passkeys.length && !passkey);
  const identityProvider = $derived(
    !!methods?.oidc &&
      (desktop ||
        (!!options?.oidc?.available &&
          options.canonical_url === location.origin)),
  );
  const proofForm = $derived(!!methods && (methods.totp || methods.password));
  function cancel() {
    controller?.abort();
    $verification?.finish(false);
  }
  async function verifyInBrowser() {
    controller = new AbortController();
    await verify(() =>
      nativeBrowserSignIn(
        controller!.signal,
        (_, code) => (pairingCode = code),
        true,
      ),
    );
    controller = null;
    pairingCode = '';
  }
  async function verify(action: () => Promise<unknown>) {
    const pending = $verification;
    busy = true;
    error = '';
    try {
      await action();
      pending?.finish(true);
      proof = '';
    } catch (caught) {
      error = errorMessage(caught);
    } finally {
      busy = false;
    }
  }
</script>

{#if $verification && methods}
  <Modal title="Verify your identity" {busy} onClose={cancel}>
    <p class="mt-0 text-xs text-muted">
      This change needs a recent sign-in. Confirm it's you to continue.
    </p>
    {#if proofForm}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void verify(() =>
            api(
              '/auth/verify',
              'POST',
              methods.totp
                ? { code: proof.replace(/\s/g, '') }
                : { password: proof },
            ),
          );
        }}
      >
        <FormField
          >{methods.totp ? 'Authentication code' : 'Password'}<input
            class={formControlClass}
            bind:value={proof}
            type={methods.totp ? 'text' : 'password'}
            inputmode={methods.totp ? 'numeric' : undefined}
            autocomplete={methods.totp ? 'one-time-code' : 'current-password'}
            maxlength={methods.totp ? 7 : undefined}
            required
            disabled={busy}
          /></FormField
        >
        <Button type="submit" disabled={busy}>Verify identity</Button>
      </form>
    {/if}
    {#if passkey || identityProvider}<div class="mt-3 flex flex-wrap gap-2">
        {#if passkey}<Button
            variant="secondary"
            disabled={busy}
            onclick={() =>
              desktop ? verifyInBrowser() : verify(() => passkeySignIn(true))}
            >Verify with passkey</Button
          >{/if}
        {#if identityProvider}<Button
            variant="secondary"
            disabled={busy}
            onclick={() => (desktop ? verifyInBrowser() : oidcSignIn('verify'))}
            >Verify with identity provider</Button
          >{/if}
      </div>{/if}
    {#if passkeyElsewhere}<p class="text-xs text-muted">
        Passkeys can't be used at this address{options &&
        options.canonical_url !== location.origin
          ? `; use ${options.canonical_url} for changes that need one`
          : '. Browsers only allow them over HTTPS with a hostname, or on localhost'}.
      </p>{/if}
    {#if !proofForm && !passkey && !identityProvider}<Notice
        role="alert"
        variant="error"
        >None of your sign-in methods can confirm your identity at this address.
        {options && options.canonical_url !== location.origin
          ? `Open ${options.canonical_url} instead`
          : 'Reach this server over HTTPS to use them'}, or ask an administrator
        for a recovery link.</Notice
      >{/if}
    {#if busy && controller}<div role="status" class="mt-4">
        {#if pairingCode}<p class="mb-1">
            Approve in your browser and enter this code:
          </p>
          <p class="my-2 font-mono text-xl font-semibold tracking-[0.2em]">
            {pairingCode}
          </p>{/if}
        <Button variant="secondary" onclick={cancel}
          >Cancel browser verification</Button
        >
      </div>{/if}
    {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  </Modal>
{/if}
