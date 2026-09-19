<script lang="ts">
  import { api } from './api';
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

<section class="panel">
  <h2>Media ownership and operations</h2>
  <p>
    Refresh ownership after a manager import or configuration change. Unresolved
    and ambiguous files cannot be deleted.
  </p>
  <button disabled={busy} onclick={() => void refresh()}
    >Reconcile file ownership</button
  >
  {#if message}<p role="status">{message}</p>{/if}
  {#each files as file (file.id)}<article>
      <p>{file.path} · <strong>{file.ownership}</strong></p>
      {#each file.bindings as binding (binding.service)}<p class="muted">
          {binding.service} · file {binding.file_id} · media {binding.entity_id}{binding
            .members.length
            ? ` · episode/track IDs ${binding.members.join(', ')}`
            : ''}
        </p>{/each}
    </article>{/each}
  {#if operations.length}<h3>Recent operations</h3>{/if}
  {#each operations as operation (operation.id)}<article>
      <p>
        {operation.title} · {operation.action} · {operation.state} · {operation.files}
        file(s)
      </p>
      {#if operation.error}<p>{operation.error}</p>{/if}
    </article>{/each}
</section>
