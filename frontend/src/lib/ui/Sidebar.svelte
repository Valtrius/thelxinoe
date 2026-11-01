<script lang="ts">
  import { PanelLeftClose, PanelLeftOpen, LogOut, X } from '@lucide/svelte';
  import SidebarButton from './SidebarButton.svelte';
  import SectionIcon from './SectionIcon.svelte';
  import AttentionDot from './AttentionDot.svelte';
  import { attention, attentionDescription } from '../attention';
  import Button from './Button.svelte';
  import type { User } from '../api';
  import {
    followLink,
    routeHref,
    sectionRoute,
    type Section,
  } from '../navigation';
  import { providers as availability } from '../providers/availability';
  let {
    section,
    collapsed,
    user,
    navigate,
    toggle,
    logout,
    onClose,
    settingsSection = 'account',
  } = $props<{
    section: Section;
    collapsed: boolean;
    user: User;
    navigate: (name: string) => void;
    toggle: () => void;
    logout: () => void;
    onClose?: () => void;
    settingsSection?: string;
  }>();
  const library = [
    'Home',
    'Discover',
    'Movies',
    'Shows',
    'Music',
    'Playlists',
  ] as const;
  const providers = [
    { name: 'YouTube', platform: 'youtube' },
    { name: 'Twitch', platform: 'twitch' },
    { name: 'Kick', platform: 'kick' },
  ] as const;
  // Linked accounts are marked on their own provider instead.
  const settingsAttention = $derived(
    $attention.filter((item) =>
      ['services', 'server', 'backups', 'jobs'].includes(item.target),
    ),
  );
  function menuAttention(name: string) {
    return $attention.filter(
      (item) =>
        item.target === name.toLowerCase() ||
        (name === 'Discover' && item.target === 'requests'),
    );
  }
</script>

<aside
  class={[
    'primary-sidebar group/sidebar relative z-30 min-h-0 min-w-0 grow-0 shrink-0',
    onClose ? 'h-full w-full' : collapsed ? 'w-18 basis-18' : 'w-45 basis-45',
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
    <div class="flex h-16 shrink-0 items-center">
      <a
        href="#home"
        data-sidebar-resize="viewport"
        class="flex h-16 min-w-0 flex-1 items-center overflow-hidden whitespace-nowrap"
        onclick={(e) => {
          if (!followLink(e)) return;
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
          class="brand-label text-[11px] font-semibold tracking-[0.12em] uppercase transition-opacity duration-(--animation-duration) group-[.collapsed]/sidebar:opacity-0"
          >Thelxinoe</span
        ></a
      >
      {#if onClose}
        <Button
          variant="ghost"
          size="icon"
          class="mr-2 size-11 shrink-0"
          aria-label="Close navigation"
          onclick={onClose}><X class="size-5" /></Button
        >
      {/if}
    </div>
    <nav
      class="primary-navigation grid min-h-0 shrink overflow-y-auto scrollbar-none"
      aria-label="Main navigation"
      data-sidebar-resize="width"
    >
      {#each library as name (name)}<SidebarButton
          label={name}
          resizeWithSidebar={false}
          {collapsed}
          title={collapsed ? name : undefined}
          aria-label={name}
          description={attentionDescription(menuAttention(name))}
          active={section === name}
          href={routeHref(sectionRoute(name))}
          onclick={(event) => {
            if (followLink(event)) {
              event.preventDefault();
              navigate(name);
            }
          }}
          ><span class="relative size-5 shrink-0"
            ><SectionIcon section={name} /><AttentionDot
              items={menuAttention(name)}
              class="absolute -top-1 -right-1.5"
            /></span
          ></SidebarButton
        >{/each}
      {#if Object.values($availability).some(Boolean)}<div
          class="h-px bg-line"
        ></div>{/if}
      {#each providers.filter((item) => $availability[item.platform]) as item (item.name)}<SidebarButton
          label={item.name}
          resizeWithSidebar={false}
          {collapsed}
          title={collapsed ? item.name : undefined}
          aria-label={item.name}
          description={attentionDescription(
            $attention.filter(
              (entry) =>
                entry.target === 'online' && entry.resource === item.platform,
            ),
          )}
          active={section === item.name}
          href={routeHref(sectionRoute(item.name))}
          onclick={(event) => {
            if (followLink(event)) {
              event.preventDefault();
              navigate(item.name);
            }
          }}
          ><span class="relative size-5 shrink-0"
            ><SectionIcon section={item.name} /><AttentionDot
              items={$attention.filter(
                (entry) =>
                  entry.target === 'online' && entry.resource === item.platform,
              )}
              class="absolute -top-1 -right-1.5"
            /></span
          ></SidebarButton
        >{/each}
    </nav>
    <div class="sidebar-bottom mt-auto shrink-0 pb-2">
      <SidebarButton
        label="Statistics"
        {collapsed}
        aria-label="Statistics"
        title={collapsed ? 'Statistics' : undefined}
        active={section === 'Statistics'}
        href={routeHref(sectionRoute('Statistics'))}
        onclick={(event) => {
          if (followLink(event)) {
            event.preventDefault();
            navigate('Statistics');
          }
        }}><SectionIcon section="Statistics" /></SidebarButton
      >
      <SidebarButton
        label="Settings"
        {collapsed}
        aria-label="Settings"
        description={attentionDescription(settingsAttention)}
        title={collapsed ? 'Settings' : undefined}
        active={section === 'Settings'}
        href={routeHref(sectionRoute('Settings', settingsSection))}
        onclick={(event) => {
          if (followLink(event)) {
            event.preventDefault();
            navigate('Settings');
          }
        }}
        ><span class="relative size-5 shrink-0"
          ><SectionIcon section="Settings" /><AttentionDot
            items={settingsAttention}
            class="absolute -top-1 -right-1.5"
          /></span
        ></SidebarButton
      >
      <div
        class="sidebar-profile relative flex h-14.5 items-center gap-2 pr-3 pl-5.25 group-[.collapsed]/sidebar:gap-0 group-[.collapsed]/sidebar:px-1.5"
      >
        <span
          class="pointer-events-none absolute inset-x-0 top-0 h-px bg-line"
          data-sidebar-resize="x"
          aria-hidden="true"
        ></span>
        <span
          class="avatar grid size-7.5 shrink-0 place-items-center overflow-hidden rounded-full border border-line bg-surface-soft text-accent group-[.collapsed]/sidebar:size-7"
          data-sidebar-resize="xy"
          title={user.username}
        >
          {#if user.avatar}<img
              src={user.avatar}
              alt=""
              class="size-full object-cover"
            />
          {:else}{user.username[0].toUpperCase()}{/if}
        </span><span
          class="profile-label pointer-events-none absolute left-14.75 w-17.75 overflow-hidden text-[11px] whitespace-nowrap transition-opacity duration-(--animation-duration) group-[.collapsed]/sidebar:opacity-0"
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
      {#if !onClose}<SidebarButton
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
        >{/if}
    </div>
  </div>
</aside>
