<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { inlineFormClass } from './ui/styles';
  let configured = $state(false),
    clientId = $state(''),
    busy = $state(false),
    message = $state('');
  onMount(() => {
    void api<{ configured: boolean }>('/admin/online/twitch')
      .then((v) => (configured = v.configured))
      .catch((e) => (message = String(e)));
  });
  async function save() {
    busy = true;
    try {
      await api('/admin/online/twitch', 'PUT', { client_id: clientId.trim() });
      configured = true;
      clientId = '';
      message = 'Twitch application saved.';
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel>
  <h2>Twitch application</h2>
  <p>
    Configure a public Twitch application for device-code sign-in. Each person
    connects their own account. Replacing the client ID requires everyone to
    reconnect.
  </p>
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <label
      >Twitch client ID<input
        bind:value={clientId}
        required
        autocomplete="off"
        placeholder={configured
          ? 'Configured — enter an ID to replace it'
          : 'Public application client ID'}
      /></label
    >
    <Button type="submit" size="form" disabled={busy}
      >Save Twitch settings</Button
    >
  </form>
  {#if message}<p role="status">{message}</p>{/if}
</Panel>
