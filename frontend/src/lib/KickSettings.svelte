<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
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
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel">
  <h2>Kick application</h2>
  <p>
    Optional application credentials provide live status and channel metadata.
    Regular users track public channels without signing in to Kick.
  </p>
  <form
    class="inline-form"
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
    <button class="primary" disabled={busy}>Save Kick settings</button>
  </form>
  {#if message}<p role="status">{message}</p>{/if}
</section>
