<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import Panel from './ui/Panel.svelte';
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

<Panel>
  <h2>OpenID Connect</h2>
  <form
    class="max-w-160"
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
    <FormField
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
    {#if redirect}<FormField
        >Redirect URI<input
          class={formControlClass}
          value={redirect}
          readonly
        /></FormField
      >{:else}<Notice variant="callout"
        >Set THELXINOE_PUBLIC_URL to the authentication hostname before enabling
        OIDC.</Notice
      >{/if}
    {#if issuer}<FormField
        >Issuer<input
          class={formControlClass}
          value={issuer}
          readonly
        /></FormField
      >{/if}
    <div class="flex gap-2">
      <Button type="submit" disabled={busy || !redirect}
        >Save identity provider</Button
      >{#if configured}<Button
          variant="secondary"
          disabled={busy}
          onclick={() =>
            act(async () => {
              await api('/admin/auth/oidc', 'DELETE');
              configured = false;
              issuer = '';
            })}>Disable identity provider</Button
        >{/if}
    </div>
  </form>
  {#if saved}<p role="status">Identity provider saved</p>{/if}
  {#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
</Panel>
