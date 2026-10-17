<script lang="ts">
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import StatusChip from '../ui/StatusChip.svelte';
  import { cardBlockClass, formControlClass } from '../ui/styles';
  import ProviderSetupInstructions from './ProviderSetupInstructions.svelte';
  import ProviderIntegrationSwitch from './ProviderIntegrationSwitch.svelte';
  import { providers } from './availability';

  let {
    platform,
    onConfigured,
  }: {
    platform: 'twitch' | 'kick';
    onConfigured?: () => void;
  } = $props();
  const definitions = {
    twitch: {
      name: 'Twitch',
      description:
        'A public Twitch application for device-code sign-in. Each person connects their own account. Replacing the client ID requires everyone to reconnect.',
      placeholder: 'Public application client ID',
      replacement: 'Configured — enter an ID to replace it',
    },
    kick: {
      name: 'Kick',
      description:
        'Optional application credentials provide live status and channel metadata. Regular users track public channels without signing in to Kick.',
      placeholder: 'Application client ID',
      replacement: 'Configured — enter credentials to replace it',
    },
  };
  const definition = $derived(definitions[platform]);
  // null until the server has answered, so the setup guide mounts in its
  // final folded or open state instead of folding after load.
  let configured = $state<boolean | null>(null),
    clientId = $state(''),
    clientSecret = $state(''),
    busy = $state(false),
    message = $state('');
  let generation = 0;

  $effect(() => {
    const selected = platform;
    const revision = ++generation;
    configured = null;
    clientId = clientSecret = message = '';
    busy = false;
    void api<{ configured: boolean }>(`/admin/online/${selected}`)
      .then((value) => {
        if (revision === generation) configured = value.configured;
      })
      .catch((error) => {
        if (revision === generation) message = String(error);
      });
    return () => {
      generation++;
    };
  });

  async function save() {
    const revision = generation;
    busy = true;
    try {
      await api(`/admin/online/${platform}`, 'PUT', {
        client_id: clientId.trim(),
        ...(platform === 'kick' ? { client_secret: clientSecret.trim() } : {}),
      });
      if (revision !== generation) return;
      configured = true;
      clientId = clientSecret = '';
      message = `${definition.name} application saved.`;
      onConfigured?.();
    } catch (error) {
      if (revision === generation) message = String(error);
    } finally {
      if (revision === generation) busy = false;
    }
  }
</script>

<SettingsCard
  title={definition.name}
  description={$providers[platform]
    ? definition.description
    : `Turn the integration on to configure a ${definition.name} application.`}
>
  {#snippet badge()}
    {#if $providers[platform] && configured !== null}
      <StatusChip tone={configured ? 'ok' : 'warn'}
        >{configured ? 'Configured' : 'Not configured'}</StatusChip
      >
    {/if}
  {/snippet}
  {#snippet actions()}
    <ProviderIntegrationSwitch {platform} />
  {/snippet}
  {#if $providers[platform]}
    {#if configured !== null}
      <div class={cardBlockClass}>
        <ProviderSetupInstructions {platform} open={!configured} />
      </div>
    {/if}
    <form
      class="{cardBlockClass} grid gap-3"
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <div
        class="grid max-w-[40rem] grid-cols-2 gap-3 compact:grid-cols-1 [&_label]:m-0"
      >
        <FormField>
          {definition.name} client ID<input
            class={formControlClass}
            bind:value={clientId}
            required
            autocomplete="off"
            placeholder={configured
              ? definition.replacement
              : definition.placeholder}
          />
        </FormField>
        {#if platform === 'kick'}
          <FormField>
            Kick client secret<input
              class={formControlClass}
              type="password"
              bind:value={clientSecret}
              required
              autocomplete="new-password"
            />
          </FormField>
        {/if}
      </div>
      <div class="flex flex-wrap items-center gap-3">
        <Button type="submit" size="form" disabled={busy}
          >Save {definition.name} settings</Button
        >
        {#if message}<p role="status" class="m-0 text-xs text-muted">
            {message}
          </p>{/if}
      </div>
    </form>
  {:else if message}
    <p role="status" class="m-0 {cardBlockClass} text-xs text-muted">
      {message}
    </p>
  {/if}
</SettingsCard>
