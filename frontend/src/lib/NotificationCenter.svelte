<script lang="ts">
  import { untrack } from 'svelte';
  import { Bell } from '@lucide/svelte';
  import SidebarButton from './ui/SidebarButton.svelte';
  import { api } from './api';
  let { revision = 0, collapsed = false } = $props<{
    revision?: number;
    collapsed?: boolean;
  }>();
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
  let container: HTMLDivElement;
  let panel = $state<HTMLElement>();
  $effect(() => {
    if (open && panel) {
      panel.style.left = `${Math.min(container.getBoundingClientRect().right + 8, innerWidth - 300)}px`;
      panel.showPopover();
    }
  });

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

<svelte:window
  onpointerdown={(event) => {
    if (
      open &&
      event.target instanceof Node &&
      !container.contains(event.target)
    )
      open = false;
  }}
  onkeydown={(event) => {
    if (open && event.key === 'Escape') {
      open = false;
      container.querySelector<HTMLButtonElement>('button')?.focus();
    }
  }}
/>
<div class="notifications" bind:this={container}>
  <SidebarButton
    label={unread ? `Notifications (${unread})` : 'Notifications'}
    {collapsed}
    aria-label={unread ? `Notifications (${unread})` : 'Notifications'}
    aria-expanded={open}
    onclick={() => (open = !open)}
    ><Bell class="size-5 shrink-0" /></SidebarButton
  >
  {#if open}<section
      bind:this={panel}
      popover="manual"
      class="panel notices"
      aria-label="Notifications"
    >
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
    position: fixed;
    inset: auto;
    bottom: 24px;
    margin: 0;
    width: min(480px, calc(100vw - 210px));
    max-height: 70vh;
    overflow: auto;
    z-index: 80;
    background: var(--surface-strong);
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
  @media (max-width: 720px) {
    .notices {
      position: fixed;
      left: 80px;
      right: 8px;
      bottom: 12px;
      width: auto;
    }
  }
</style>
