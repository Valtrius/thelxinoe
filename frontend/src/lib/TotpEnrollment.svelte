<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { Check, Copy } from '@lucide/svelte';
  import { api, ApiError, desktop } from './api';
  import {
    type TotpSetup,
    errorMessage,
    withVerification,
  } from './authentication';
  import { copyText } from './clipboard';
  import Modal from './ui/Modal.svelte';
  import QrCode from './ui/QrCode.svelte';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let {
    setup: initial,
    onClose,
    onEnabled,
  } = $props<{
    setup: TotpSetup;
    onClose: () => void;
    onEnabled: () => void;
  }>();
  let setup = $state(untrack(() => initial));
  let code = $state(''),
    busy = $state(false),
    error = $state(''),
    hint = $state(''),
    restartable = $state(false),
    enabled = $state(false);
  let copied = $state(false),
    copyError = $state('');
  let content: HTMLDivElement;
  let codeInput = $state<HTMLInputElement>();
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => () => clearTimeout(copiedTimer));
  const groupedSecret = $derived(
    setup.secret.match(/.{1,4}/g)?.join(' ') ?? setup.secret,
  );
  // otpauth: links only help where an authenticator app is installed on this device.
  const handheld =
    !desktop &&
    typeof matchMedia === 'function' &&
    matchMedia('(pointer: coarse)').matches;

  async function copySecret() {
    copyError = '';
    try {
      await copyText(setup.secret, content);
      copied = true;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = false), 2500);
    } catch {
      copyError =
        "Couldn't copy automatically. Select the setup key and copy it manually.";
    }
  }
  async function restart() {
    busy = true;
    error = hint = '';
    restartable = false;
    try {
      await withVerification(async () => {
        setup = await api<TotpSetup>('/me/auth/totp/start', 'POST');
        code = '';
      });
    } catch (caught) {
      error = errorMessage(caught);
    } finally {
      busy = false;
    }
  }
  async function confirm() {
    error = hint = '';
    restartable = false;
    const digits = code.replace(/\s/g, '');
    if (!/^\d{6}$/.test(digits)) {
      error = 'Enter the 6-digit code shown in your authenticator app.';
      codeInput?.focus();
      return;
    }
    code = digits;
    busy = true;
    try {
      enabled = await withVerification(async () => {
        await api('/me/auth/totp/confirm', 'POST', { code: digits });
      });
    } catch (caught) {
      error = errorMessage(caught);
      const reason = caught instanceof ApiError ? caught.code : '';
      if (reason === 'totp_incorrect') {
        hint =
          'Make sure you scanned the QR code shown here (not an older one) and that the time on your device is set automatically.';
        code = '';
      } else if (reason === 'totp_reused') code = '';
      else if (reason === 'totp_setup_missing') restartable = true;
    } finally {
      busy = false;
    }
    if (enabled) return;
    await tick();
    codeInput?.focus();
  }
</script>

<Modal
  title="Set up authenticator app"
  {busy}
  onClose={enabled ? onEnabled : onClose}
>
  <div bind:this={content} class="text-xs leading-normal">
    {#if enabled}
      <p role="status" class="mt-0 text-sm font-semibold text-success">
        Authenticator app enabled
      </p>
      <p class="text-muted">
        Signing in with your password now also asks for a code from the app.
        Your other devices were signed out and need to sign in again. Passkeys
        and your identity provider sign in without a code.
      </p>
      <div class="mt-5 flex justify-end">
        <Button onclick={onEnabled}>Done</Button>
      </div>
    {:else}
      <p class="mt-0 text-muted">
        Use an authenticator app such as 1Password, Bitwarden, Aegis, Google
        Authenticator or Microsoft Authenticator. Once it is enabled, signing in
        with your password also asks for the app's 6-digit code.
      </p>
      <h3 class="mt-5 mb-3 text-sm font-semibold">1. Scan this QR code</h3>
      <div class="flex items-start gap-5 compact:flex-col">
        <div class="w-56 max-w-full shrink-0">
          <QrCode
            value={setup.uri}
            label={`Authenticator QR code for ${setup.account}`}
          />
        </div>
        <div class="min-w-0 flex-1">
          <p class="mt-0">
            Can't scan it? Choose to enter a setup key in the app and use this
            key:
          </p>
          <div class="flex items-center gap-2">
            <input
              class="{formControlClass} font-mono tracking-wider"
              readonly
              value={groupedSecret}
              aria-label="Setup key"
              onfocus={(event) => event.currentTarget.select()}
            />
            <Button
              variant="secondary"
              size="icon"
              class="shrink-0"
              aria-label={copied ? 'Setup key copied' : 'Copy setup key'}
              title={copied ? 'Copied' : 'Copy setup key'}
              onclick={copySecret}
              >{#if copied}<Check size={16} />{:else}<Copy
                  size={16}
                />{/if}</Button
            >
          </div>
          {#if copied}<p role="status" class="mt-1.5 mb-0 text-success">
              Setup key copied
            </p>{/if}
          {#if copyError}<p role="alert" class="mt-1.5 mb-0 text-danger">
              {copyError}
            </p>{/if}
          <dl
            class="mt-3 mb-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-muted [&_dd]:m-0 [&_dd]:wrap-anywhere [&_dd]:text-foreground"
          >
            <dt>Account</dt>
            <dd>{setup.account}</dd>
            <dt>Issuer</dt>
            <dd>{setup.issuer}</dd>
            <dt>Type</dt>
            <dd>
              Time-based · {setup.digits} digits · every {setup.period} seconds ·
              {setup.algorithm}
            </dd>
          </dl>
          {#if handheld}<a
              class="mt-3 inline-block text-accent"
              href={setup.uri}>Open in an authenticator app on this device</a
            >{/if}
        </div>
      </div>
      <form
        novalidate
        onsubmit={(event) => {
          event.preventDefault();
          void confirm();
        }}
      >
        <h3 class="mt-6 mb-3 text-sm font-semibold">
          2. Enter the code from the app
        </h3>
        <FormField
          >Authentication code<input
            bind:this={codeInput}
            class="{formControlClass} font-mono tracking-[0.3em]"
            bind:value={code}
            inputmode="numeric"
            autocomplete="one-time-code"
            maxlength="7"
            aria-invalid={!!error}
            disabled={busy}
          /></FormField
        >
        {#if error}<Notice role="alert" variant="error"
            >{error}{#if hint}<p class="mt-1.5 mb-0">{hint}</p>{/if}</Notice
          >{/if}
        <div class="flex flex-wrap justify-end gap-2">
          {#if restartable}<Button
              variant="secondary"
              class="mr-auto"
              disabled={busy}
              onclick={restart}>Generate a new QR code</Button
            >{/if}
          <Button variant="secondary" disabled={busy} onclick={onClose}
            >Cancel</Button
          >
          <Button type="submit" loading={busy}>Enable authenticator</Button>
        </div>
      </form>
    {/if}
  </div>
</Modal>
