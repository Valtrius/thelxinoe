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
    ['devices', 'Devices', 'Sessions and Quick Connect'],
  ];
  const native = [
    ['mpv', 'MPV', 'Player, configuration and plugins'],
    ['connection', 'Connection', 'Choose your server'],
  ];
  const admin = [
    ['server', 'Server', 'Health and current activity'],
    ['library', 'Metadata', 'Matching and artwork providers'],
    ['providers', 'Online accounts', 'YouTube, Twitch and Kick apps'],
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
    {#each [...personal, ...(desktop ? native : []), ...(desktop || user.role === 'admin' ? [['updates', 'Updates', 'Product releases and recovery']] : []), ...(user.role === 'admin' ? admin : [])] as [id, label, description] (id)}
      <NavigationItem
        aria-label={label}
        active={active === id}
        onclick={() => (active = id)}
        class="settings-nav-item"
        ><span>{label}</span><small>{description}</small></NavigationItem
      >
    {/each}
  </nav>
  <div class="settings-content" data-sidebar-resize-origin>
    {@render children()}
  </div>
</div>
