<script lang="ts">
  import { api } from './api';
  let { id, changed } = $props<{ id: string; changed: () => void }>();
  let busy = $state(false),
    message = $state(''),
    pending = $state<{ id: string; files: number; action: string } | null>(
      null,
    );
  async function act(fn: () => Promise<void>) {
    busy = true;
    message = '';
    try {
      await fn();
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
  async function prepare(action: string) {
    pending = null;
    const result = await api<{ id: string; files: number }>(
      '/admin/media/operations',
      'POST',
      { media_id: id, action },
    );
    pending = { ...result, action };
  }
</script>

<section class="panel">
  <h3>Manage media</h3>
  <button
    class="secondary"
    disabled={busy}
    onclick={() =>
      void act(async () => {
        await api(`/admin/media/${id}/keep`, 'PUT', { keep: true });
        message = 'Keep protection enabled.';
      })}>Keep media</button
  >
  <button
    class="secondary"
    disabled={busy}
    onclick={() =>
      void act(async () => {
        await api(`/admin/media/${id}/keep`, 'PUT', { keep: false });
        message = 'Keep protection removed.';
      })}>Remove Keep</button
  >
  <button
    class="secondary"
    disabled={busy}
    onclick={() => void act(() => prepare('monitor'))}>Monitor files</button
  >
  <button
    class="secondary"
    disabled={busy}
    onclick={() => void act(() => prepare('unmonitor'))}>Unmonitor files</button
  >
  <button
    class="secondary"
    disabled={busy}
    onclick={() => void act(() => prepare('delete'))}
    >Prepare file deletion</button
  >
  {#if pending}<p>
      {pending.action} · {pending.files} file(s). Ownership and protection will be
      checked again before this runs.
    </p>
    <button
      class="secondary"
      disabled={busy}
      onclick={() =>
        void act(async () => {
          const operation = pending;
          if (!operation) return;
          pending = null;
          await api(`/admin/media/operations/${operation.id}/execute`, 'POST');
          message = 'Operation completed.';
          changed();
        })}>Confirm {pending.action}</button
    >
    <button class="secondary" disabled={busy} onclick={() => (pending = null)}
      >Cancel</button
    >
  {/if}
  {#if message}<p role="status">{message}</p>{/if}
</section>
