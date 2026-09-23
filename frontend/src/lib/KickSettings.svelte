<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass } from './ui/styles';
  import ProviderSetupInstructions from './providers/ProviderSetupInstructions.svelte';
  let { onConfigured }: { onConfigured?: () => void } = $props();
  let configured = $state(false),
    clientId = $state(''),
    clientSecret = $state(''),
    busy = $state(false),
    message = $state('');
  onMount(() => {
    void api<{ configured: boolean }>('/admin/online/kick')
      .then((v) => (configured = v.configured))
      .catch((e) => (message = String(e)));
  });
  async function save() {
    busy = true;
    try {
      await api('/admin/online/kick', 'PUT', {
        client_id: clientId.trim(),
        client_secret: clientSecret.trim(),
      });
      configured = true;
      clientId = '';
      clientSecret = '';
      message = 'Kick application saved.';
      onConfigured?.();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel>
  <h2>Kick application</h2>
  <p>
    Optional application credentials provide live status and channel metadata.
    Regular users track public channels without signing in to Kick.
  </p>
  <ProviderSetupInstructions platform="kick" />
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <label
      >Kick client ID<input
        bind:value={clientId}
        required
        autocomplete="off"
        placeholder={configured
          ? 'Configured — enter credentials to replace it'
          : 'Application client ID'}
      /></label
    >
    <label
      >Kick client secret<input
        type="password"
        bind:value={clientSecret}
        required
        autocomplete="new-password"
      /></label
    >
    <Button type="submit" size="form" disabled={busy}>Save Kick settings</Button
    >
  </form>
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
