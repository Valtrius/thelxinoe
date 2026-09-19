<script lang="ts">
  import { api } from './api';
  let { username } = $props<{ username: string }>();
  let code = $state('');
  let device = $state<{
    device: string;
    client: string;
    version: string;
    confirmation: string;
  } | null>(null);
  let error = $state(''),
    busy = $state(false),
    approved = $state(false);
  async function inspect() {
    busy = true;
    error = '';
    approved = false;
    device = null;
    try {
      device = await api('/auth/quick-connect/inspect', 'POST', { code });
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function approve() {
    if (!device) return;
    busy = true;
    error = '';
    try {
      await api('/auth/quick-connect/approve', 'POST', {
        code,
        confirmation: device.confirmation,
      });
      approved = true;
      device = null;
      code = '';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="panel" aria-label="Quick Connect">
  <h2>Connect a TV</h2>
  <p>Enter the Quick Connect code displayed by your media client.</p>
  <form
    class="inline-form"
    onsubmit={(event) => {
      event.preventDefault();
      void inspect();
    }}
  >
    <label
      >TV code<input
        bind:value={code}
        oninput={() => {
          device = null;
          approved = false;
        }}
        pattern={'[0-9]{6}'}
        maxlength="6"
        inputmode="numeric"
        autocomplete="off"
        required
      /></label
    >
    <button disabled={busy}>Find device</button>
  </form>
  {#if device}<p>
      Sign in to <strong>{device.device}</strong> using {device.client}
      {device.version} as <strong>{username}</strong>.
    </p>
    <button class="primary" disabled={busy} onclick={() => void approve()}
      >Connect this device</button
    >{/if}
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  {#if approved}<p role="status">Device approved. Continue on your TV.</p>{/if}
</section>
