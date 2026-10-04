<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import { captureSession } from './session';
  import Panel from './ui/Panel.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass, rowClass } from './ui/styles';
  type Device = { id: string; name: string; last_seen: number };
  let devices = $state<Device[]>([]),
    clients = $state<Device[]>([]),
    name = $state('');
  let busy = $state(false),
    error = $state(''),
    password = $state('');
  onMount(() => {
    void load().catch((e) => (error = String(e)));
  });
  async function load() {
    const owns = captureSession();
    const [remembered, app] = await Promise.all([
      api<{ items: Device[] }>('/auth/devices'),
      api<{ items: Device[] }>('/me/auth/client-passwords'),
    ]);
    if (owns()) {
      devices = remembered.items;
      clients = app.items;
    }
  }
  async function act(action: () => Promise<void>) {
    const owns = captureSession();
    busy = true;
    error = '';
    try {
      await withVerification(action);
      if (owns()) await load();
    } catch (caught) {
      if (owns()) error = String(caught);
    } finally {
      if (owns()) busy = false;
    }
  }
</script>

<Panel>
  <h2>Remembered devices</h2>
  {#each devices as device (device.id)}<div class={rowClass}>
      <strong class="min-w-0 flex-1 truncate">{device.name}</strong><Button
        variant="secondary"
        disabled={busy}
        aria-label={`Forget ${device.name}`}
        onclick={() =>
          act(async () => {
            await api(`/auth/devices/${device.id}`, 'DELETE');
          })}>Forget device</Button
      >
    </div>{:else}<p class="text-muted">No remembered devices</p>{/each}
</Panel>
<Panel>
  <h2>Client passwords</h2>
  <p class="text-muted">
    For compatible media clients. These passwords bypass TOTP and can only
    access the Jellyfin-compatible API.
  </p>
  {#each clients as client (client.id)}<div class={rowClass}>
      <strong class="min-w-0 flex-1 truncate">{client.name}</strong><Button
        variant="secondary"
        disabled={busy}
        aria-label={`Revoke ${client.name}`}
        onclick={() =>
          act(async () => {
            await api(`/me/auth/client-passwords/${client.id}`, 'DELETE');
          })}>Revoke</Button
      >
    </div>{/each}
  <form
    class="max-w-120"
    onsubmit={(e) => {
      e.preventDefault();
      void act(async () => {
        password = (
          await api<{ password: string }>('/me/auth/client-passwords', 'POST', {
            name,
          })
        ).password;
        name = '';
      });
    }}
  >
    <FormField
      >Client name<input
        class={formControlClass}
        bind:value={name}
        maxlength="100"
        required
        disabled={busy}
      /></FormField
    ><Button type="submit" disabled={busy}>Create client password</Button>
  </form>
  {#if password}<FormField
      >New client password<input
        class={formControlClass}
        value={password}
        readonly
      /></FormField
    >
    <p class="text-muted">Copy this password now. It is shown once.</p>
    <Button variant="secondary" onclick={() => (password = '')}>Done</Button
    >{/if}
</Panel>
{#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
