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
    ['account', 'Account', 'Profile and appearance'],
    ['playback', 'Playback', 'Quality, languages and skipping'],
    ['online', 'Online accounts', 'Your connections and saved data'],
    ['devices', 'Devices', 'Sessions and Quick Connect'],
  ];
  const native = [
    ['mpv', 'MPV', 'Player, configuration and plugins'],
    ['connection', 'Connection', 'Choose your server'],
    ['updates', 'Desktop updates', 'Update this application'],
  ];
  const admin = [
    ['server', 'Server', 'Health and current activity'],
    ['server-updates', 'Server updates', 'Product releases and recovery'],
    ['library', 'Metadata', 'Matching and artwork providers'],
    ['analysis', 'Episode analysis', 'Intro and credit detection'],
    ['providers', 'Provider applications', 'YouTube, Twitch and Kick apps'],
    ['services', 'Media services', 'Installation and integrations'],
    ['retention', 'Retention', 'Cleanup policies and protection'],
    ['backups', 'Backups', 'Encrypted archives and recovery'],
    ['people', 'People', 'Users and permissions'],
    ['jobs', 'Activity', 'Background jobs'],
    ['audit', 'Audit', 'Administrative actions'],
  ];
</script>

<div class="settings-layout">
  <nav aria-label="Settings navigation" class="settings-navigation">
    <h2 class="settings-nav-heading">User settings</h2>
    {#each [...personal, ...(desktop ? native : [])] as [id, label, description] (id)}
      <NavigationItem
        aria-label={label}
        active={active === id}
        onclick={() => (active = id)}
        class="settings-nav-item"
        ><span>{label}</span><small>{description}</small></NavigationItem
      >
    {/each}
    {#if user.role === 'admin'}
      <hr class="settings-nav-divider" />
      <h2 class="settings-nav-heading">Administration</h2>
      {#each admin as [id, label, description] (id)}
        <NavigationItem
          aria-label={label}
          active={active === id}
          onclick={() => (active = id)}
          class="settings-nav-item"
          ><span>{label}</span><small>{description}</small></NavigationItem
        >
      {/each}
    {/if}
  </nav>
  <div class="settings-content" data-sidebar-resize-origin>
    <div class="settings-panels">{@render children()}</div>
  </div>
</div>
