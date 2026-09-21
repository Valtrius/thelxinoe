<script lang="ts">
  import type { Snippet } from 'svelte';
  import NavigationItem from './NavigationItem.svelte';
  import { desktop, type User } from '../api';
  let {
    user,
    active = $bindable('account'),
    children,
  } = $props<{ user: User; active?: string; children: Snippet }>();
  const personal = [
    ['account', 'Account'],
    ['playback', 'Playback'],
    ['online', 'Online accounts'],
    ['devices', 'Devices'],
  ];
  const native = [
    ['mpv', 'MPV'],
    ['connection', 'Connection'],
    ['updates', 'Desktop updates'],
  ];
  const admin = [
    ['server', 'Server'],
    ['server-updates', 'Server updates'],
    ['analysis', 'Episode analysis'],
    ['providers', 'Provider applications'],
    ['services', 'Media services'],
    ['retention', 'Retention'],
    ['backups', 'Backups'],
    ['people', 'People'],
    ['jobs', 'Activity'],
    ['audit', 'Audit'],
  ];
  const groups = $derived([
    {
      id: 'user',
      heading: user.role === 'admin' ? 'User settings' : null,
      items: [...personal, ...(desktop ? native : [])],
    },
    ...(user.role === 'admin'
      ? [{ id: 'administration', heading: 'Administration', items: admin }]
      : []),
  ]);
</script>

<div class="settings-layout flex min-h-full items-start compact:flex-col">
  <nav
    aria-label="Settings navigation"
    class="settings-navigation sticky top-0 z-1 flex h-(--workspace-height) max-h-(--workspace-height) w-47.5 shrink-0 flex-col overflow-y-auto border-r border-line bg-surface py-3 narrow:w-38.75 compact:h-auto compact:max-h-none compact:w-full compact:flex-row compact:overflow-x-auto compact:border-r-0 compact:border-b compact:p-0"
  >
    {#each groups as group, index (group.id)}
      {#if index > 0}
        <hr
          class="my-0 h-px shrink-0 border-0 bg-line compact:mx-0 compact:h-auto compact:w-px compact:self-stretch"
        />
      {/if}
      {#if group.heading}<h2
          class="m-0 shrink-0 px-4.5 py-2 text-[10px] font-semibold tracking-[0.1em] text-muted uppercase compact:flex compact:items-center compact:px-3.5 compact:py-0 compact:whitespace-nowrap"
        >
          {group.heading}
        </h2>{/if}
      {#each group.items as [id, label] (id)}
        <NavigationItem
          aria-label={label}
          active={active === id}
          onclick={() => (active = id)}
          class="settings-nav-item shrink-0 px-4.5 py-3.75 narrow:p-3.25 compact:p-3.5 compact:whitespace-nowrap"
          ><span class="text-[11px] font-semibold tracking-[0.08em] uppercase"
            >{label}</span
          ></NavigationItem
        >
      {/each}
    {/each}
  </nav>
  <div
    class="settings-content min-w-0 flex-1 p-6 narrow:p-4 compact:min-h-0 compact:w-full compact:p-3"
    data-sidebar-resize-origin
  >
    <div class="settings-panels w-full max-w-220">{@render children()}</div>
  </div>
</div>
