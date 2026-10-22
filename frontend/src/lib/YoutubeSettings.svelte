<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import ProviderSetupInstructions from './providers/ProviderSetupInstructions.svelte';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import CopyButton from './ui/CopyButton.svelte';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ProviderIntegrationSwitch from './providers/ProviderIntegrationSwitch.svelte';
  import { providers } from './providers/availability';
  import type { YoutubeDownloadQuality } from './providers/types';
  let { onConfigured }: { onConfigured?: () => void } = $props();
  type Family = 'ipv4' | 'ipv6';
  type Network = 'auto' | Family;
  type Configuration = {
    google_configured: boolean;
    redirect_uri: string | null;
    youtube_downloads: boolean;
    youtube_daily_quota: number;
    youtube_network: Network;
    youtube_download_quality: YoutubeDownloadQuality;
    youtube_network_status: {
      ipv4: boolean;
      ipv6: boolean;
      current: Family | null;
    };
    quota: { used: number; blocked: boolean };
  };
  const familyLabels: Record<Family, string> = { ipv4: 'IPv4', ipv6: 'IPv6' };
  const ids = $props.id();
  let config = $state<Configuration | null>(null),
    clientId = $state(''),
    clientSecret = $state(''),
    downloads = $state(false),
    budget = $state(10000),
    network = $state<Network>('auto'),
    downloadQuality = $state<YoutubeDownloadQuality>('best'),
    busy = $state(false),
    savingPreferences = $state(false),
    message = $state('');
  async function load() {
    config = await api<Configuration>('/admin/online');
    downloads = config.youtube_downloads;
    budget = config.youtube_daily_quota;
    network = config.youtube_network;
    downloadQuality = config.youtube_download_quality;
  }
  // The server reports no current family only when a fixed family has no route.
  function networkSummary(config: Configuration) {
    const { current, ipv6 } = config.youtube_network_status;
    if (config.youtube_network !== 'auto')
      return current
        ? `Public videos use ${familyLabels[current]}.`
        : `${familyLabels[config.youtube_network]} is not available to this server.`;
    return `Public videos currently use ${familyLabels[current ?? 'ipv4']}.${
      ipv6
        ? ''
        : ' IPv6 is not available to this server, so it cannot replace a blocked IPv4 address.'
    }`;
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
        youtube_network: network,
        youtube_download_quality: downloadQuality,
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

<SettingsCard
  title="YouTube"
  description={$providers.youtube
    ? 'One Google Web application for this server. Each person connects their own YouTube account. Replacing the application credentials requires everyone to reconnect.'
    : 'Turn the integration on to configure a Google application and let people connect YouTube.'}
  autoSave={$providers.youtube && Boolean(config)}
  saving={savingPreferences}
>
  {#snippet badge()}
    {#if $providers.youtube && config}
      <StatusChip tone={config.google_configured ? 'ok' : 'warn'}
        >{config.google_configured
          ? 'Configured'
          : 'Not configured'}</StatusChip
      >
    {/if}
  {/snippet}
  {#snippet actions()}
    <ProviderIntegrationSwitch platform="youtube" />
  {/snippet}
  {#if $providers.youtube}
    <!-- The guide mounts once the configuration is known so it starts folded
         for a configured application instead of folding after load. -->
    {#if config}
      <div class={cardBlockClass}>
        <ProviderSetupInstructions
          platform="youtube"
          redirectUri={config.redirect_uri}
          open={!config.google_configured}
        />
      </div>
    {/if}
    {#if config?.redirect_uri}
      <SettingRow
        label="Authorized redirect URI"
        description="Add this exact URI to the Web application client in Google Cloud."
        for={`${ids}-redirect`}
      >
        <div class="flex w-72 max-w-full items-center gap-2 compact:w-full">
          <input
            id={`${ids}-redirect`}
            class="{formControlClass} font-mono text-[11px] text-muted"
            readonly
            value={config.redirect_uri}
            onclick={(event) => event.currentTarget.select()}
          /><CopyButton value={config.redirect_uri} label="Copy redirect URI" />
        </div>
      </SettingRow>
    {:else}<p role="status" class="m-0 {cardBlockClass} text-xs text-muted">
        Set THELXINOE_PUBLIC_URL to this server's HTTPS origin in its
        environment and restart the server to get the redirect URI. Configure
        THELXINOE_TRUSTED_PROXIES for your HTTPS reverse proxy. HTTP localhost
        is supported for development.
      </p>{/if}
    <form
      class="{cardBlockClass} grid gap-3"
      onsubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div>
        <div class="text-[13px] leading-snug font-[550]">
          Google credentials
        </div>
        <p class="mt-0.75 mb-0 text-xs text-muted">
          Client ID and secret of the Web application client.
        </p>
      </div>
      <div
        class="grid max-w-[40rem] grid-cols-2 gap-3 compact:grid-cols-1 [&_label]:m-0"
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
      </div>
      <div>
        <Button
          type="submit"
          size="form"
          disabled={busy || savingPreferences || !config}
          >Apply Google application</Button
        >
      </div>
    </form>
    {#if config}<AutoSaveForm
        label="YouTube preferences"
        class="divide-y divide-line"
        bind:busy={savingPreferences}
        disabled={busy || !config}
        value={{
          youtube_downloads: downloads,
          youtube_daily_quota: budget,
          youtube_network: network,
          youtube_download_quality: downloadQuality,
        }}
        onRevert={(previous) => {
          downloads = previous.youtube_downloads;
          budget = previous.youtube_daily_quota;
          network = previous.youtube_network;
          downloadQuality = previous.youtube_download_quality;
        }}
        onsave={async (submitted) => {
          await api('/admin/online', 'PUT', submitted);
          config = await api<Configuration>('/admin/online');
        }}
      >
        <SettingRow
          label="Daily API budget"
          description={`${config.quota.used.toLocaleString()} API units used today. The shared budget resets at midnight Pacific time.${config.quota.blocked ? ' YouTube has paused requests for today.' : ''}`}
          for={`${ids}-budget`}
          control="field"
        >
          <input
            id={`${ids}-budget`}
            class={formControlClass}
            type="number"
            bind:value={budget}
            min="1"
            max="10000000"
            required
          />
        </SettingRow>
        <SettingRow
          label="Public video network"
          description={`Automatic switches to IPv6 when YouTube blocks the IPv4 address. ${networkSummary(config)}`}
          for={`${ids}-network`}
          control="field"
        >
          <select
            id={`${ids}-network`}
            class={formControlClass}
            bind:value={network}
          >
            <option value="auto">Automatic</option>
            <option value="ipv4">IPv4 only</option>
            <option value="ipv6">IPv6 only</option>
          </select>
        </SettingRow>
        <SettingRow label="Allow YouTube downloads" for={`${ids}-downloads`}>
          <Switch id={`${ids}-downloads`} bind:checked={downloads} />
        </SettingRow>
        {#if downloads}<SettingRow
            label="Download quality"
            for={`${ids}-download-quality`}
            control="field"
          >
            <select
              id={`${ids}-download-quality`}
              class={formControlClass}
              bind:value={downloadQuality}
            >
              <option value="best">Best available</option>
              {#each ['2160p', '1440p', '1080p', '720p'] as quality (quality)}<option
                  value={quality}>Up to {quality}</option
                >{/each}
            </select>
          </SettingRow>{/if}
      </AutoSaveForm>{/if}
    {#if message}<p
        role="status"
        class="m-0 {cardBlockClass} text-xs text-muted"
      >
        {message}
      </p>{/if}
  {/if}
</SettingsCard>
