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
  import Button from './Button.svelte';
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

<aside
  class={[
    'primary-sidebar group/sidebar relative z-30 min-h-0 min-w-0 grow-0 shrink-0',
    collapsed
      ? 'w-18 basis-18'
      : 'w-45 basis-45 compact:absolute compact:z-70 compact:h-full compact:bg-background',
  ]}
  class:collapsed
  aria-label="Application sidebar"
>
  <div
    data-sidebar-resize="x"
    aria-hidden="true"
    class="sidebar-surface pointer-events-none absolute inset-0 z-0 origin-left border-r border-line bg-surface"
  ></div>
  <div class="relative flex h-full min-h-0 flex-col overflow-visible">
    <a
      href="#home"
      class="flex h-16 shrink-0 items-center overflow-hidden whitespace-nowrap"
      onclick={(e) => {
        e.preventDefault();
        navigate('Home');
      }}
      aria-label="Thelxinoe home"
      ><img
        class="mx-5.25 block size-7.5 shrink-0 object-contain"
        src="/icon.svg"
        alt=""
        width="30"
        height="30"
      /><span
        class="brand-label text-[11px] font-semibold tracking-[0.12em] uppercase transition-opacity duration-150 group-[.collapsed]/sidebar:w-0 group-[.collapsed]/sidebar:basis-0 group-[.collapsed]/sidebar:opacity-0"
        >Thelxinoe</span
      ></a
    >
    <nav
      class="primary-navigation grid min-h-0 shrink overflow-y-auto scrollbar-none"
      aria-label="Main navigation"
      data-sidebar-resize="width"
    >
      {#each library as item (item.name)}<SidebarButton
          label={item.name}
          resizeWithSidebar={false}
          {collapsed}
          title={collapsed ? item.name : undefined}
          aria-label={item.name}
          active={section === item.name}
          onclick={() => navigate(item.name)}
          ><item.icon class="size-5 shrink-0" /></SidebarButton
        >{/each}
      <div class="h-px bg-line"></div>
      {#each providers as item (item.name)}<SidebarButton
          label={item.name}
          resizeWithSidebar={false}
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
      <div class="h-px bg-line"></div>
      <SidebarButton
        label="Requests"
        resizeWithSidebar={false}
        {collapsed}
        aria-label="Requests"
        title={collapsed ? 'Requests' : undefined}
        active={section === 'Requests'}
        onclick={() => navigate('Requests')}
        ><Search class="size-5 shrink-0" /></SidebarButton
      >
    </nav>
    <div class="sidebar-bottom mt-auto shrink-0 pb-2">
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
      {#if !desktop}<div
          class="web-theme-controls relative flex justify-center py-2 group-[.collapsed]/sidebar:[--compact-choice-width:24px]"
        >
          <span
            class="pointer-events-none absolute inset-x-0 top-0 h-px bg-line"
            data-sidebar-resize="x"
            aria-hidden="true"
          ></span>
          <ThemeControls resizeWithSidebar />
        </div>{/if}
      <div
        class="sidebar-profile relative flex h-14.5 items-center gap-2 pr-3 pl-5.25 group-[.collapsed]/sidebar:gap-0 group-[.collapsed]/sidebar:px-1.5"
      >
        <span
          class="pointer-events-none absolute inset-x-0 top-0 h-px bg-line"
          data-sidebar-resize="x"
          aria-hidden="true"
        ></span>
        <span
          class="avatar grid size-7.5 shrink-0 place-items-center rounded-full border border-line bg-surface-soft text-accent group-[.collapsed]/sidebar:size-7"
          data-sidebar-resize="xy"
          title={user.username}>{user.username[0].toUpperCase()}</span
        ><span
          class="profile-label pointer-events-none absolute left-14.75 w-17.75 overflow-hidden text-[11px] whitespace-nowrap transition-opacity duration-150 group-[.collapsed]/sidebar:opacity-0"
          ><strong class="block truncate">{user.username}</strong><small
            class="mt-0.5 text-[9px]"
            >{user.role === 'admin' ? 'Administrator' : 'Member'}</small
          ></span
        ><Button
          variant="ghost"
          size="compact-icon"
          class="ml-auto"
          data-sidebar-resize="x-pos"
          title="Sign out"
          aria-label="Sign out"
          onclick={logout}><LogOut size={15} /></Button
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
