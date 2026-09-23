<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import ProviderSetupInstructions from './providers/ProviderSetupInstructions.svelte';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import { inlineFormClass } from './ui/styles';
  let { onConfigured }: { onConfigured?: () => void } = $props();
  type Configuration = {
    google_configured: boolean;
    redirect_uri: string | null;
    youtube_downloads: boolean;
    youtube_daily_quota: number;
    quota: { used: number; blocked: boolean };
  };
  let config = $state<Configuration | null>(null),
    clientId = $state(''),
    clientSecret = $state(''),
    downloads = $state(false),
    budget = $state(10000),
    busy = $state(false),
    savingPreferences = $state(false),
    message = $state('');
  async function load() {
    config = await api<Configuration>('/admin/online');
    downloads = config.youtube_downloads;
    budget = config.youtube_daily_quota;
  }
  onMount(() => {
    void load().catch((e) => (message = String(e)));
  });
  async function save() {
    busy = true;
    message = '';
    try {
      await api('/admin/online', 'PUT', {
        ...(clientId || clientSecret
          ? {
              google: {
                client_id: clientId.trim(),
                client_secret: clientSecret.trim(),
              },
            }
          : {}),
        youtube_downloads: downloads,
        youtube_daily_quota: budget,
      });
      clientId = '';
      clientSecret = '';
      await load();
      message = 'Online provider settings saved.';
      onConfigured?.();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel>
  <h2>YouTube application</h2>
  <p class="text-muted">
    Configure one Google Web application for this server. Each person connects
    their own YouTube account. Replacing the application credentials requires
    everyone to reconnect.
  </p>
  <ProviderSetupInstructions platform="youtube" />
  {#if config?.redirect_uri}
    <FormField
      >Authorized redirect URI<input
        class={formControlClass}
        readonly
        value={config.redirect_uri}
      /></FormField
    >
  {:else}<p role="status">
      Set THELXINOE_PUBLIC_URL to this server's HTTPS origin in its environment
      and restart the server to get the redirect URI. Configure
      THELXINOE_TRUSTED_PROXIES for your HTTPS reverse proxy. HTTP localhost is
      supported for development.
    </p>{/if}
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <FormField
      >Google client ID<input
        class={formControlClass}
        bind:value={clientId}
        required
        autocomplete="off"
        placeholder={config?.google_configured
          ? 'Configured — enter credentials to replace it'
          : 'Web application client ID'}
      /></FormField
    >
    <FormField
      >Google client secret<input
        class={formControlClass}
        type="password"
        bind:value={clientSecret}
        required
        autocomplete="new-password"
        placeholder="Enter with the client ID"
      /></FormField
    >
    <Button
      type="submit"
      size="form"
      disabled={busy || savingPreferences || !config}
      >Apply Google application</Button
    >
  </form>
  {#if config}<AutoSaveForm
      label="YouTube preferences"
      class={inlineFormClass}
      bind:busy={savingPreferences}
      disabled={busy || !config}
      value={{ youtube_downloads: downloads, youtube_daily_quota: budget }}
      onRevert={(previous) => {
        downloads = previous.youtube_downloads;
        budget = previous.youtube_daily_quota;
      }}
      onsave={(submitted) => api('/admin/online', 'PUT', submitted)}
    >
      <FormField
        >Daily API budget<input
          class={formControlClass}
          type="number"
          bind:value={budget}
          min="1"
          max="10000000"
          required
        /></FormField
      >
      <Switch bind:checked={downloads}>Allow YouTube downloads</Switch>
    </AutoSaveForm>{/if}
  {#if config}<p class="text-muted">
      {config.quota.used.toLocaleString()} API units used today. The shared budget
      resets at midnight Pacific time.{config.quota.blocked
        ? ' YouTube has paused requests for today.'
        : ''}
    </p>{/if}
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
