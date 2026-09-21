<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import TwitchSettings from './TwitchSettings.svelte';
  import KickSettings from './KickSettings.svelte';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass } from './ui/styles';
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
  {#if config?.redirect_uri}
    <label
      >Authorized redirect URI<input
        readonly
        value={config.redirect_uri}
      /></label
    >
    <p class="text-muted">
      Add this exact address to the Web application in Google Cloud, enable the
      YouTube Data API v3, and grant your test users access while the
      application is in testing.
    </p>
  {:else}<p role="status">
      Set the server's public HTTPS URL before connecting accounts. HTTP
      localhost is supported for development.
    </p>{/if}
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <label
      >Google client ID<input
        bind:value={clientId}
        autocomplete="off"
        placeholder={config?.google_configured
          ? 'Configured — enter credentials to replace it'
          : 'Web application client ID'}
      /></label
    >
    <label
      >Google client secret<input
        type="password"
        bind:value={clientSecret}
        autocomplete="new-password"
        placeholder="Enter with the client ID"
      /></label
    >
    <label
      >Daily API budget<input
        type="number"
        bind:value={budget}
        min="1"
        max="10000000"
        required
      /></label
    >
    <Switch bind:checked={downloads}>Allow YouTube downloads</Switch>
    <Button type="submit" size="form" disabled={busy || !config}
      >Save YouTube settings</Button
    >
  </form>
  {#if config}<p class="text-muted">
      {config.quota.used.toLocaleString()} API units used today. The shared budget
      resets at midnight Pacific time.{config.quota.blocked
        ? ' YouTube has paused requests for today.'
        : ''}
    </p>{/if}
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
<TwitchSettings />
<KickSettings />
