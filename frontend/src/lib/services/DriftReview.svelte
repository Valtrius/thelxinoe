<script lang="ts">
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';

  let { serviceId, changed } = $props<{
    serviceId: string;
    changed: () => Promise<void> | void;
  }>();
  type Change = {
    path: string;
    change: 'added' | 'removed' | 'changed';
    before?: string | null;
    after?: string | null;
  };
  type Review = {
    revision: string;
    changes: Change[];
    blocked: string | null;
  };
  let review = $state<Review | null>(null),
    busy = $state(false),
    error = $state('');
  async function load() {
    busy = true;
    error = '';
    try {
      review = await api<Review>(`/admin/stack/${serviceId}/drift`);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function accept() {
    if (!review) return;
    busy = true;
    error = '';
    try {
      await api(`/admin/stack/${serviceId}/action`, 'POST', {
        action: 'accept_configuration',
        revision: review.revision,
      });
      review = null;
      await changed();
    } catch (e) {
      // A later change needs its own review.
      error = String(e);
      review = null;
    } finally {
      busy = false;
    }
  }
</script>

<div class="my-2 text-[11px] leading-[1.6]" aria-label="Changed configuration">
  {#if !review}
    <Button
      variant="secondary"
      size="sm"
      disabled={busy}
      onclick={() => void load()}>Review changes</Button
    >
  {:else}
    <ul class="my-2 space-y-1">
      {#each review.changes as item (item.path)}
        <li class="wrap-anywhere">
          <code>{item.path}</code>
          {item.change}{#if item.before || item.after}:
            <span class="text-muted">{item.before ?? 'unset'}</span> →
            <span>{item.after ?? 'unset'}</span>{/if}
        </li>
      {:else}
        <li class="text-muted">No differences remain.</li>
      {/each}
    </ul>
    <p class="text-muted">Environment and label values are not shown.</p>
    {#if review.blocked}
      <Notice tone="danger" class="mt-2" role="alert"
        >Thelxinoe can't own this configuration: {review.blocked}. Release
        ownership to keep the container as it is.</Notice
      >
    {:else}
      <Button size="sm" disabled={busy} onclick={() => void accept()}
        >Accept current configuration</Button
      >
    {/if}
    <Button
      variant="ghost"
      size="sm"
      disabled={busy}
      onclick={() => (review = null)}>Cancel</Button
    >
  {/if}
  {#if error}<p class="text-danger" role="alert">{error}</p>{/if}
</div>
