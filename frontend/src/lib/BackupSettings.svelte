<script lang="ts">
  import Switch from './providers/components/ui/Switch.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
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

<section class="panel" aria-label="Backups">
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
  <label
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
  <button
    class="primary"
    disabled={busy || !confirmation || passphrase.length < 16}
    onclick={() => void run()}>Create encrypted backup</button
  >
  <button class="secondary" onclick={() => void load()}>Refresh backups</button>
  {#if message}<p role="status">{message}</p>{/if}
  {#each items as item (item.id)}<div class="row">
      <div>
        <strong
          >{new Date(item.created_at * 1000).toLocaleString()} · {item.stage}</strong
        ><small>{item.archive}</small>{#if item.error}<p>{item.error}</p>{/if}
      </div>
      {#if ['complete', 'restored', 'restore-failed', 'imported'].includes(item.stage)}<button
          class="secondary"
          onclick={() => {
            restoreId = item.id;
            restoreConfirmation = '';
          }}>Restore this backup</button
        >{/if}
    </div>{/each}
  {#if restoreId}<div class="restore">
      <p>
        Restoring replaces server and managed-service state with the selected
        backup. Changes since that backup will be lost. It does not undo changes
        to media or external services.
      </p>
      <label
        >Type RESTORE to confirm<input
          bind:value={restoreConfirmation}
          autocomplete="off"
        /></label
      ><button
        class="secondary"
        disabled={busy ||
          !confirmation ||
          passphrase.length < 16 ||
          restoreConfirmation !== 'RESTORE'}
        onclick={() => void run(true)}>Restore selected state</button
      ><button class="secondary" onclick={() => (restoreId = '')}>Cancel</button
      >
    </div>{/if}
</section>

<style>
  label {
    display: block;
    margin: 1rem 0;
    max-width: 34rem;
  }
  button {
    margin: 0.3rem;
  }
  .restore {
    border: 1px solid var(--line);
    padding: 1rem;
  }
  small {
    display: block;
    overflow-wrap: anywhere;
  }
</style>
