<script lang="ts">
  import {
    verification,
    passkeySignIn,
    oidcSignIn,
    nativeBrowserSignIn,
  } from './authentication';
  import { api, desktop } from './api';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let proof = $state(''),
    busy = $state(false),
    error = $state('');
  let controller = $state<AbortController | null>(null);
  $effect(() => {
    if (!$verification) {
      controller?.abort();
      proof = '';
      error = '';
    }
  });
  function cancel() {
    controller?.abort();
    $verification?.finish(false);
  }
  async function verifyInBrowser() {
    controller = new AbortController();
    await verify(() => nativeBrowserSignIn(controller!.signal, () => {}, true));
    controller = null;
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
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

{#if $verification}
  <Modal title="Verify your identity" {busy} onClose={cancel}>
    {#if $verification.methods.totp || $verification.methods.password}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void verify(() =>
            api(
              '/auth/verify',
              'POST',
              $verification?.methods.totp
                ? { code: proof }
                : { password: proof },
            ),
          );
        }}
      >
        <FormField
          >{$verification.methods.totp
            ? 'Authentication code'
            : 'Password'}<input
            class={formControlClass}
            bind:value={proof}
            type={$verification.methods.totp ? 'text' : 'password'}
            inputmode={$verification.methods.totp ? 'numeric' : undefined}
            autocomplete={$verification.methods.totp
              ? 'one-time-code'
              : 'current-password'}
            required
            disabled={busy}
          /></FormField
        >
        <Button type="submit" disabled={busy}>Verify identity</Button>
      </form>
    {/if}
    {#if $verification.methods.passkeys.length}
      <Button
        variant="secondary"
        disabled={busy}
        onclick={() =>
          desktop ? verifyInBrowser() : verify(() => passkeySignIn('', true))}
        >Verify with passkey</Button
      >
    {/if}
    {#if $verification.methods.oidc}
      <Button
        variant="secondary"
        disabled={busy}
        onclick={() => (desktop ? verifyInBrowser() : oidcSignIn('verify'))}
        >Verify with identity provider</Button
      >
    {/if}
    {#if busy && controller}<Button variant="secondary" onclick={cancel}
        >Cancel browser verification</Button
      >{/if}
    {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
  </Modal>
{/if}
