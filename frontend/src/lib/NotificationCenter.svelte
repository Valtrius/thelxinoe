<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  let { revision = 0 } = $props<{ revision?: number }>();
  type Notice = {
    id: string;
    severity: string;
    message: string;
    created_at: number;
    read_at: number | null;
  };
  let items = $state<Notice[]>([]),
    open = $state(false),
    error = $state('');
  const unread = $derived(items.filter((n) => !n.read_at).length);
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  async function load() {
    try {
      items = (await api<{ items: Notice[] }>('/me/notifications')).items;
    } catch (e) {
      error = String(e);
    }
  }
  async function mark(id: string) {
    try {
      await api(`/me/notifications/${id}`, 'PUT');
      await load();
    } catch (e) {
      error = String(e);
    }
  }
</script>

<div class="notifications">
  <button class="secondary" aria-expanded={open} onclick={() => (open = !open)}
    >Notifications{unread ? ` (${unread})` : ''}</button
  >
  {#if open}<section class="panel notices" aria-label="Notifications">
      <div class="section-heading">
        <h2>Notifications</h2>
        <button class="secondary" onclick={() => void mark('all')}
          >Mark all read</button
        >
      </div>
      {#if error}<p role="alert">{error}</p>{/if}
      {#each items as item (item.id)}<article class:unread={!item.read_at}>
          <p>{item.message}</p>
          <small
            >{item.severity} · {new Date(
              item.created_at * 1000,
            ).toLocaleString()}</small
          >
          {#if !item.read_at}<button
              class="secondary"
              onclick={() => void mark(item.id)}>Mark read</button
            >{/if}
        </article>{:else}<p>No notifications.</p>{/each}
    </section>{/if}
</div>

<style>
  .notifications {
    position: relative;
  }
  .notices {
    position: absolute;
    right: 0;
    top: 100%;
    width: min(480px, 85vw);
    max-height: 70vh;
    overflow: auto;
    z-index: 80;
    background: var(--panel);
  }
  article {
    padding: 1rem 0;
    border-top: 1px solid var(--line);
  }
  article.unread {
    border-left: 3px solid var(--accent);
    padding-left: 1rem;
  }
  article p {
    margin: 0.2rem 0;
  }
  article button {
    margin-left: 0.6rem;
  }
  small {
    color: var(--muted);
  }
</style>
