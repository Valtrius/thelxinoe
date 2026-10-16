<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import SettingsCard from './ui/SettingsCard.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import FormField from './ui/FormField.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import { formControlClass } from './ui/styles';
  let discovery = $state(''),
    clientId = $state(''),
    secret = $state(''),
    label = $state('Single sign-on'),
    issuer = $state(''),
    redirect = $state('');
  let configured = $state(false),
    busy = $state(false),
    error = $state(''),
    saved = $state(false);
  async function load() {
    const value = await api<{
      provider: {
        discovery_url: string;
        client_id: string;
        label: string;
        issuer: string;
      } | null;
      redirect_uri: string | null;
    }>('/admin/auth/oidc');
    configured = !!value.provider;
    redirect = value.redirect_uri ?? '';
    if (value.provider) {
      discovery = value.provider.discovery_url;
      clientId = value.provider.client_id;
      label = value.provider.label;
      issuer = value.provider.issuer;
    }
  }
  onMount(() => {
    void load().catch((e) => (error = String(e)));
  });
  async function act(action: () => Promise<void>) {
    busy = true;
    error = '';
    saved = false;
    try {
      if (!(await withVerification(action))) return;
      await load();
      secret = '';
      saved = true;
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
</script>

<SettingsCard
  id="server-identity"
  title="OpenID Connect"
  description="Let people sign in through your identity provider. The sign-in button label appears on the sign-in page."
  padded
>
  {#snippet badge()}
    <StatusChip tone={configured ? 'ok' : 'neutral'}
      >{configured ? 'Configured' : 'Not configured'}</StatusChip
    >
  {/snippet}
  <form
    class="grid max-w-[52rem] gap-4"
    onsubmit={(e) => {
      e.preventDefault();
      void act(async () => {
        await api('/admin/auth/oidc', 'POST', {
          discovery_url: discovery,
          client_id: clientId,
          client_secret: secret || undefined,
          label,
        });
      });
    }}
  >
    <div
      class="grid grid-cols-2 gap-4 compact:grid-cols-1 [&_label]:m-0 [&_label]:max-w-none"
    >
      <FormField class="col-span-full"
        >Discovery URL<input
          class={formControlClass}
          type="url"
          bind:value={discovery}
          required
          disabled={busy}
          placeholder="https://auth.example.com/.well-known/openid-configuration"
        /></FormField
      >
      <FormField
        >Client ID<input
          class={formControlClass}
          bind:value={clientId}
          required
          disabled={busy}
        /></FormField
      >
      <FormField
        >Client secret<input
          class={formControlClass}
          type="password"
          bind:value={secret}
          required={!configured}
          disabled={busy}
          autocomplete="new-password"
          placeholder={configured ? 'Leave empty to keep the saved secret' : ''}
        /></FormField
      >
      <FormField
        >Sign-in button label<input
          class={formControlClass}
          bind:value={label}
          required
          maxlength="100"
          disabled={busy}
        /></FormField
      >
      {#if issuer}<FormField
          >Issuer<input
            class={formControlClass}
            value={issuer}
            readonly
          /></FormField
        >{/if}
      {#if redirect}<FormField class="col-span-full"
          >Redirect URI<input
            class={formControlClass}
            value={redirect}
            readonly
          /></FormField
        >{:else}<Notice variant="callout" class="col-span-full mb-0"
          >Set THELXINOE_PUBLIC_URL to the authentication hostname before
          enabling OIDC.</Notice
        >{/if}
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <Button type="submit" size="form" disabled={busy || !redirect}
        >Save identity provider</Button
      >{#if configured}<Button
          variant="danger"
          size="form"
          disabled={busy}
          onclick={() =>
            act(async () => {
              await api('/admin/auth/oidc', 'DELETE');
              configured = false;
              issuer = '';
            })}>Disable identity provider</Button
        >{/if}
      {#if saved}<p role="status" class="m-0 text-xs text-muted">
          Identity provider saved
        </p>{/if}
    </div>
  </form>
  {#if error}<Notice role="alert" variant="error" class="mt-4 mb-0"
      >{error}</Notice
    >{/if}
</SettingsCard>
