<script lang="ts">
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Panel from '../ui/Panel.svelte';
  import { formControlClass, inlineFormClass } from '../ui/styles';
  import ProviderSetupInstructions from './ProviderSetupInstructions.svelte';

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
        'Configure a public Twitch application for device-code sign-in. Each person connects their own account. Replacing the client ID requires everyone to reconnect.',
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
  let configured = $state(false),
    clientId = $state(''),
    clientSecret = $state(''),
    busy = $state(false),
    message = $state('');
  let generation = 0;

  $effect(() => {
    const selected = platform;
    const revision = ++generation;
    configured = false;
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

<Panel>
  <h2>{definition.name} application</h2>
  <p>{definition.description}</p>
  <ProviderSetupInstructions {platform} />
  <form
    class={inlineFormClass}
    onsubmit={(event) => {
      event.preventDefault();
      void save();
    }}
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
    <Button type="submit" size="form" disabled={busy}
      >Save {definition.name} settings</Button
    >
  </form>
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
