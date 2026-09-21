<script lang="ts">
  import Switch from './ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { rowClass } from './ui/styles';
  type Backup = {
    id: string;
    stage: string;
    created_at: number;
    error: string | null;
    archive: string;
  };
  let items = $state<Backup[]>([]),
    passphrase = $state(''),
    confirmation = $state(false),
    restoreId = $state(''),
    restoreConfirmation = $state(''),
    message = $state(''),
    busy = $state(false),
    destination = $state('');
  async function load() {
    try {
      const data = await api<{ items: Backup[]; destination: string }>(
        '/admin/backups',
      );
      items = data.items;
      destination = data.destination;
    } catch (e) {
      message = String(e);
    }
  }
  onMount(() => {
    void load();
    const timer = setInterval(() => void load(), 10000);
    return () => clearInterval(timer);
  });
  async function run(restore = false) {
    busy = true;
    message = '';
    try {
      await api(
        restore ? `/admin/backups/${restoreId}/restore` : '/admin/backups',
        'POST',
        { passphrase, confirm: confirmation },
      );
      passphrase = '';
      restoreConfirmation = '';
      message =
        'Operation started. The server will briefly disconnect while state is copied. Refresh after it reconnects; restoration may require signing in again.';
      await load();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Panel aria-label="Backups">
  <h2>Backups and restore</h2>
  <p>
    Backups include server state, credentials, deployment information and
    managed-service appdata. Media and cache are excluded. Services briefly stop
    so their databases can be copied consistently.
  </p>
  <p>
    Archives are encrypted with your passphrase and saved in {destination ||
      'the controller deployment backup directory'}. Keep the passphrase
    separately; Thelxinoe cannot recover it.
  </p>
  <label class="my-4 block max-w-136"
    >Backup passphrase<input
      type="password"
      autocomplete="new-password"
      minlength="16"
      bind:value={passphrase}
    /></label
  >
  <Switch bind:checked={confirmation}
    >I understand that services will temporarily stop.</Switch
  >
  <Button
    size="form"
    class="m-[0.3rem]"
    disabled={busy || !confirmation || passphrase.length < 16}
    onclick={() => void run()}>Create encrypted backup</Button
  >
  <Button
    variant="secondary"
    size="form"
    class="m-[0.3rem]"
    onclick={() => void load()}>Refresh backups</Button
  >
  {#if message}<p role="status">{message}</p>{/if}
  {#each items as item (item.id)}<div class={rowClass}>
      <div>
        <strong
          >{new Date(item.created_at * 1000).toLocaleString()} · {item.stage}</strong
        ><small class="block wrap-anywhere">{item.archive}</small
        >{#if item.error}<p>{item.error}</p>{/if}
      </div>
      {#if ['complete', 'restored', 'restore-failed', 'imported'].includes(item.stage)}<Button
          variant="secondary"
          size="form"
          class="m-[0.3rem]"
          onclick={() => {
            restoreId = item.id;
            restoreConfirmation = '';
          }}>Restore this backup</Button
        >{/if}
    </div>{/each}
  {#if restoreId}<div class="border border-line p-4">
      <p>
        Restoring replaces server and managed-service state with the selected
        backup. Changes since that backup will be lost. It does not undo changes
        to media or external services.
      </p>
      <label class="my-4 block max-w-136"
        >Type RESTORE to confirm<input
          bind:value={restoreConfirmation}
          autocomplete="off"
        /></label
      ><Button
        variant="secondary"
        size="form"
        class="m-[0.3rem]"
        disabled={busy ||
          !confirmation ||
          passphrase.length < 16 ||
          restoreConfirmation !== 'RESTORE'}
        onclick={() => void run(true)}>Restore selected state</Button
      ><Button
        variant="secondary"
        size="form"
        class="m-[0.3rem]"
        onclick={() => (restoreId = '')}>Cancel</Button
      >
    </div>{/if}
</Panel>
