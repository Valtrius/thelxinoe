<script lang="ts">
  import type { Snippet } from 'svelte';
  import {
    Activity,
    AppWindow,
    AudioLines,
    ArchiveRestore,
    Boxes,
    Clapperboard,
    Hourglass,
    Laptop,
    Link,
    MonitorSmartphone,
    Play,
    ScrollText,
    Search,
    Server,
    UserRound,
    UsersRound,
  } from '@lucide/svelte';
  import NavigationItem from './NavigationItem.svelte';
  import { desktop, type User } from '../api';
  import AttentionDot from './AttentionDot.svelte';
  import { attention, attentionDescription } from '../attention';
  import { formControlClass } from './styles';
  import { providers } from '../providers/availability';
  let {
    user,
    active = $bindable('account'),
    children,
  } = $props<{ user: User; active?: string; children: Snippet }>();
  let query = $state('');
  const personal = [
    {
      id: 'account',
      label: 'Account',
      icon: UserRound,
      keywords:
        'profile picture avatar password display timezone time format workspace density watched',
    },
    {
      id: 'playback',
      label: 'Playback',
      icon: Play,
      keywords:
        'video quality audio language subtitles replaygain intro recap credits preview skipping',
    },
    {
      id: 'online',
      label: 'Online accounts',
      icon: Link,
      keywords: 'youtube twitch kick connect synchronization',
    },
    {
      id: 'devices',
      label: 'Devices',
      icon: MonitorSmartphone,
      keywords: 'quick connect sessions revoke access',
    },
  ];
  const native = [
    {
      id: 'mpv',
      label: 'MPV',
      icon: Clapperboard,
      keywords: 'player configuration shortcuts',
    },
    {
      id: 'connection',
      label: 'Desktop',
      icon: Laptop,
      keywords: 'server address desktop updates version',
    },
  ];
  const admin = [
    {
      id: 'server',
      label: 'Server',
      icon: Server,
      keywords:
        'health storage cache docker tools updates timezone time format diagnostics',
    },
    {
      id: 'analysis',
      label: 'Episode analysis',
      icon: AudioLines,
      keywords: 'intro credits timestamps theintrodb detection',
    },
    {
      id: 'providers',
      label: 'Provider applications',
      icon: AppWindow,
      keywords: 'youtube twitch kick client secret oauth api quota downloads',
    },
    {
      id: 'services',
      label: 'Media services',
      icon: Boxes,
      keywords:
        'seerr recyclarr radarr sonarr lidarr bazarr prowlarr nzbget downloads requests profiles trash guides indexers updates ownership',
    },
    {
      id: 'retention',
      label: 'Retention',
      icon: Hourglass,
      keywords: 'watched deletion grace period movies tv seasons library roots',
    },
    {
      id: 'backups',
      label: 'Backups',
      icon: ArchiveRestore,
      keywords: 'restore encryption archives passphrase',
    },
    {
      id: 'people',
      label: 'People',
      icon: UsersRound,
      keywords: 'users permissions roles password',
    },
    {
      id: 'jobs',
      label: 'Activity',
      icon: Activity,
      keywords: 'jobs background checkpoint failed running queued completed',
    },
    {
      id: 'audit',
      label: 'Audit',
      icon: ScrollText,
      keywords: 'administrative history actions users',
    },
  ];
  const groups = $derived([
    {
      label: 'Personal',
      items: [
        ...personal.filter(
          (item) =>
            item.id !== 'online' || Object.values($providers).some(Boolean),
        ),
        ...(desktop ? native : []),
      ],
    },
    ...(user.role === 'admin'
      ? [{ label: 'Administration', items: admin }]
      : []),
  ]);
  const title = $derived(
    groups.flatMap((group) => group.items).find((item) => item.id === active)
      ?.label ?? 'Settings',
  );
  const filtered = $derived(
    groups.map((group) => ({
      ...group,
      items: group.items.filter((item) =>
        (item.label + ' ' + item.keywords)
          .toLowerCase()
          .includes(query.trim().toLowerCase()),
      ),
    })),
  );
</script>

<div
  class="settings-layout mx-auto flex min-h-full w-full max-w-520 items-start gap-8 px-8 py-7 narrow:gap-5 narrow:px-5 compact:flex-col compact:gap-6 compact:px-4 compact:py-4"
>
  <nav
    aria-label="Settings navigation"
    class="settings-navigation @container/settings-nav sticky top-0 z-1 max-h-[calc(var(--workspace-height)-2rem)] w-49 overflow-y-auto shrink-0 space-y-6 py-1 compact:static compact:max-h-none compact:w-full compact:overflow-visible compact:space-y-4"
  >
    <label class="relative block">
      <span class="sr-only">Find a setting</span>
      <Search
        size={14}
        class="pointer-events-none absolute top-3 left-2.5 text-muted"
        aria-hidden="true"
      />
      <input
        type="search"
        class={[formControlClass, 'pl-8 text-xs']}
        placeholder="Find a setting"
        bind:value={query}
      />
    </label>
    {#each filtered as group (group.label)}
      {#if group.items.length}
        <div>
          <p
            class="mb-2 px-2 text-[10px] font-semibold tracking-widest text-muted uppercase"
          >
            {group.label}
          </p>
          <div
            class="grid gap-1 compact:grid-cols-3 compact:gap-1.5 compact:@max-[270px]/settings-nav:grid-cols-2"
          >
            {#each group.items as item (item.id)}
              <NavigationItem
                aria-label={item.label}
                description={attentionDescription(
                  $attention.filter((entry) => entry.target === item.id),
                )}
                active={active === item.id}
                onclick={() => {
                  active = item.id;
                  query = '';
                }}
                class="settings-nav-item flex min-h-10 items-center gap-2.5 border border-transparent px-2.5 py-2 text-xs aria-[current=page]:border-line-strong compact:min-h-11 compact:gap-1 compact:px-1 compact:text-[11px] [&_[data-nav-accent]]:hidden"
              >
                <item.icon
                  size={16}
                  strokeWidth={1.6}
                  class="shrink-0 compact:size-3.5"
                  aria-hidden="true"
                />
                <span class="min-w-0 leading-snug wrap-anywhere"
                  >{item.label}</span
                >
                <AttentionDot
                  items={$attention.filter((entry) => entry.target === item.id)}
                />
              </NavigationItem>
            {/each}
          </div>
        </div>
      {/if}
    {/each}
    {#if !filtered.some((group) => group.items.length)}<p
        role="status"
        class="text-xs text-muted"
      >
        No matching settings.
      </p>{/if}
  </nav>
  <div
    class="settings-content @container/settings min-w-0 flex-1 compact:w-full"
    data-sidebar-resize-origin
  >
    <h1 class="mb-7 text-[25px] font-semibold tracking-tight compact:mb-5">
      {title}
    </h1>
    <div class="settings-panels settings-grid">{@render children()}</div>
  </div>
</div>
