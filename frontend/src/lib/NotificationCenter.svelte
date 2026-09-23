<script lang="ts">
  import SectionHeading from './ui/SectionHeading.svelte';
  import { untrack } from 'svelte';
  import { Bell } from '@lucide/svelte';
  import SidebarButton from './ui/SidebarButton.svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';

  let {
    revision = 0,
    collapsed = false,
    timezone,
    timeFormat,
  } = $props<{
    revision?: number;
    collapsed?: boolean;
    timezone: string;
    timeFormat: '12h' | '24h';
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
<div class="notifications relative" bind:this={container}>
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
      class="panel notices fixed inset-auto bottom-6 z-80 m-0 max-h-[70vh] w-[min(480px,calc(100vw-210px))] overflow-auto border border-line bg-surface-strong p-6 shadow-none compact:right-2 compact:bottom-3 compact:left-20 compact:w-auto compact:p-4"
      aria-label="Notifications"
    >
      <SectionHeading>
        <h2>Notifications</h2>
        <Button variant="secondary" size="form" onclick={() => void mark('all')}
          >Mark all read</Button
        >
      </SectionHeading>
      {#if error}<p role="alert">{error}</p>{/if}
      {#each items as item (item.id)}<article
          class:unread={!item.read_at}
          class={item.read_at
            ? 'border-t border-line py-4'
            : 'border-t border-l-3 border-line border-l-accent py-4 pl-4'}
        >
          <p class="my-[0.2rem]">{item.message}</p>
          <small class="text-muted"
            >{item.severity} · {new Date(item.created_at * 1000).toLocaleString(
              undefined,
              {
                timeZone: timezone,
                hour12: timeFormat === '12h',
              },
            )}</small
          >
          {#if !item.read_at}<Button
              variant="secondary"
              size="form"
              class="ml-[0.6rem]"
              onclick={() => void mark(item.id)}>Mark read</Button
            >{/if}
        </article>{:else}<p>No notifications.</p>{/each}
    </section>{/if}
</div>
