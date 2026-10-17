<script lang="ts">
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import { cardBlockClass } from './ui/styles';
  type File = {
    id: string;
    path: string;
    ownership: string;
    bindings: {
      service: string;
      file_id: number;
      entity_id: number;
      members: number[];
    }[];
  };
  type Operation = {
    id: string;
    title: string;
    action: string;
    state: string;
    files: number;
    error: string | null;
  };
  let files = $state<File[]>([]),
    operations = $state<Operation[]>([]),
    busy = $state(false),
    message = $state('');
  async function refresh() {
    busy = true;
    message = '';
    try {
      await api('/admin/managers/reconcile', 'POST');
      files = (await api<{ items: File[] }>('/admin/managers/bindings')).items;
      operations = (
        await api<{ items: Operation[] }>('/admin/media/operations')
      ).items;
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<SettingsCard
  title="Library files"
  description="Auto-delete only removes files whose manager is known. Reconcile after a Radarr, Sonarr or Lidarr import or configuration change; unresolved files are never deleted."
>
  <SettingRow
    label="Reconcile file ownership"
    description="Re-reads which manager owns each library file and lists any recent media operations."
  >
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void refresh()}>Reconcile file ownership</Button
    >
  </SettingRow>
  {#if message}<p role="status" class="m-0 {cardBlockClass} text-xs text-muted">
      {message}
    </p>{/if}
  {#each files as file (file.id)}
    <div class="{cardBlockClass} text-xs">
      <p class="m-0 wrap-anywhere">
        {file.path} · <strong>{file.ownership}</strong>
      </p>
      {#each file.bindings as binding (binding.service)}<p
          class="mt-1 mb-0 text-muted"
        >
          {binding.service} · file {binding.file_id} · media {binding.entity_id}{binding
            .members.length
            ? ` · episode/track IDs ${binding.members.join(', ')}`
            : ''}
        </p>{/each}
    </div>
  {/each}
  {#if operations.length}
    <div class="{cardBlockClass} text-xs">
      <h3 class="mb-2 text-xs font-semibold">Recent operations</h3>
      {#each operations as operation (operation.id)}
        <p class="mb-1 last:mb-0">
          {operation.title} · {operation.action} · {operation.state} · {operation.files}
          file(s){#if operation.error}
            <span class="block text-danger">{operation.error}</span>{/if}
        </p>
      {/each}
    </div>
  {/if}
</SettingsCard>
