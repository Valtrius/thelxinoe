<script lang="ts">
  import {
    House,
    Film,
    Tv,
    Music,
    ListMusic,
    History,
    Search,
    ChartNoAxesCombined,
    PanelLeftClose,
    PanelLeftOpen,
    Settings,
    LogOut,
  } from '@lucide/svelte';
  import SidebarButton from './SidebarButton.svelte';
  import PlatformIcon from './PlatformIcon.svelte';
  import NotificationCenter from '../NotificationCenter.svelte';
  import ThemeControls from './ThemeControls.svelte';
  import { desktop, type User } from '../api';
  let {
    section,
    collapsed,
    user,
    navigate,
    toggle,
    logout,
    notificationRevision,
  } = $props<{
    notificationRevision: number;
    section: string;
    collapsed: boolean;
    user: User;
    navigate: (name: string) => void;
    toggle: () => void;
    logout: () => void;
  }>();
  const library = [
    { name: 'Home', icon: House },
    { name: 'Movies', icon: Film },
    { name: 'Shows', icon: Tv },
    { name: 'Music', icon: Music },
    { name: 'Playlists', icon: ListMusic },
  ];
  const providers = [
    { name: 'YouTube', platform: 'youtube' },
    { name: 'Twitch', platform: 'twitch' },
    { name: 'Kick', platform: 'kick' },
  ] as const;
</script>

<aside class="primary-sidebar" class:collapsed aria-label="Application sidebar">
  <div data-sidebar-resize="x" aria-hidden="true" class="sidebar-surface"></div>
  <div class="sidebar-inner">
    <a
      href="#home"
      class="sidebar-brand"
      onclick={(e) => {
        e.preventDefault();
        navigate('Home');
      }}
      aria-label="Thelxinoe home"
      ><span class="brand-symbol">T</span><span class="brand-label"
        >Thelxinoe</span
      ></a
    >
    <nav class="primary-navigation" aria-label="Main navigation">
      {#each library as item (item.name)}<SidebarButton
          label={item.name}
          {collapsed}
          title={collapsed ? item.name : undefined}
          aria-label={item.name}
          active={section === item.name}
          onclick={() => navigate(item.name)}
          ><item.icon class="size-5 shrink-0" /></SidebarButton
        >{/each}
      <div class="sidebar-divider" data-sidebar-resize="x"></div>
      {#each providers as item (item.name)}<SidebarButton
          label={item.name}
          {collapsed}
          title={collapsed ? item.name : undefined}
          aria-label={item.name}
          active={section === item.name}
          onclick={() => navigate(item.name)}
          ><PlatformIcon
            platform={item.platform}
            class="size-5 shrink-0"
          /></SidebarButton
        >{/each}
      <SidebarButton
        label="Requests"
        {collapsed}
        aria-label="Requests"
        title={collapsed ? 'Requests' : undefined}
        active={section === 'Requests'}
        onclick={() => navigate('Requests')}
        ><Search class="size-5 shrink-0" /></SidebarButton
      >
    </nav>
    <div class="sidebar-bottom">
      <SidebarButton
        label="History"
        {collapsed}
        aria-label="History"
        title={collapsed ? 'History' : undefined}
        active={section === 'History'}
        onclick={() => navigate('History')}
        ><History class="size-5 shrink-0" /></SidebarButton
      >
      <SidebarButton
        label="Statistics"
        {collapsed}
        aria-label="Statistics"
        title={collapsed ? 'Statistics' : undefined}
        active={section === 'Statistics'}
        onclick={() => navigate('Statistics')}
        ><ChartNoAxesCombined class="size-5 shrink-0" /></SidebarButton
      >
      <SidebarButton
        label="Settings"
        {collapsed}
        aria-label="Settings"
        title={collapsed ? 'Settings' : undefined}
        active={section === 'Settings'}
        onclick={() => navigate('Settings')}
        ><Settings class="size-5 shrink-0" /></SidebarButton
      >
      <NotificationCenter revision={notificationRevision} {collapsed} />
      {#if !desktop}<div class="web-theme-controls">
          <span class="sidebar-rule" data-sidebar-resize="x" aria-hidden="true"
          ></span>
          <ThemeControls resizeWithSidebar />
        </div>{/if}
      <div class="sidebar-profile">
        <span class="sidebar-rule" data-sidebar-resize="x" aria-hidden="true"
        ></span>
        <span class="avatar" data-sidebar-resize="xy" title={user.username}
          >{user.username[0].toUpperCase()}</span
        ><span class="profile-label"
          ><strong>{user.username}</strong><small
            >{user.role === 'admin' ? 'Administrator' : 'Member'}</small
          ></span
        ><button
          class="icon-button"
          data-sidebar-resize="x-pos"
          title="Sign out"
          aria-label="Sign out"
          onclick={logout}><LogOut size={15} /></button
        >
      </div>
      <SidebarButton
        label="Collapse"
        {collapsed}
        size="sm"
        title={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        onclick={toggle}
        >{#if collapsed}<PanelLeftOpen
            class="size-4 shrink-0"
          />{:else}<PanelLeftClose
            class="size-4 shrink-0"
          />{/if}</SidebarButton
      >
    </div>
  </div>
</aside>
