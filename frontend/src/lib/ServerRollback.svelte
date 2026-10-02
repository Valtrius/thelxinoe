<script lang="ts">
  import { Undo2 } from '@lucide/svelte';
  import { serverUpdates, serverUpdateAction } from './server-updates';
  import Button from './ui/Button.svelte';
  import Modal from './ui/Modal.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  let open = $state(false);
  let confirmation = $state('');
  const status = $derived($serverUpdates.status);
  // A snapshot can restore only its own accepted deployment generation.
  const recovery = $derived(
    status?.controller.items.find(
      (item) =>
        item.snapshot_ready &&
        item.version === status.version &&
        ['committed', 'runtime-failure', 'recovery-required'].includes(
          item.stage,
        ),
    ),
  );
</script>

{#if recovery}
  <Button
    variant="ghost"
    size="sm"
    onclick={() => {
      confirmation = '';
      open = true;
    }}><Undo2 size={14} />Rollback</Button
  >
{/if}
{#if open && recovery}
  <Modal
    title={`Rollback to ${recovery.previous_version}`}
    onClose={() => (open = false)}
  >
    <p class="text-sm leading-relaxed">
      Restores the server and its data to the snapshot before {recovery.version}.
      Changes since that update will be lost.
    </p>
    <p class="mt-2 text-xs text-muted">
      Media files and external services are unchanged.
    </p>
    <div class="mt-5 grid gap-3">
      <FormField
        >Type RESTORE to continue<input
          class={formControlClass}
          bind:value={confirmation}
          autocomplete="off"
        /></FormField
      >
      <Button
        variant="danger"
        size="form"
        loading={$serverUpdates.action === 'recover'}
        disabled={confirmation !== 'RESTORE' || Boolean($serverUpdates.action)}
        onclick={async () => {
          if (await serverUpdateAction('recover', recovery.id)) open = false;
        }}>Restore {recovery.previous_version}</Button
      >
      {#if $serverUpdates.error}<p role="alert" class="text-xs text-danger">
          {$serverUpdates.error}
        </p>{/if}
    </div>
  </Modal>
{/if}
