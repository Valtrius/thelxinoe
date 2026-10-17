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
  import { followLink, routeHref, sectionRoute } from '../navigation';
  import AttentionDot from './AttentionDot.svelte';
  import { attention, attentionDescription } from '../attention';
  import { activity } from '../activity';
  import { formControlClass } from './styles';
  import { providers } from '../providers/availability';
  let {
    user,
    active = 'account',
    children,
    subheader,
    openSettings,
  } = $props<{
    user: User;
    active?: string;
    children: Snippet;
    subheader?: Snippet;
    openSettings: (name: string) => void;
  }>();
  let query = $state('');
  let scroller = $state<HTMLElement>();
  // The navigation and page header stay put; only the cards scroll. A new
  // section starts at the top. The scroller is positioned so absolutely
  // placed descendants (screen-reader text, badges) scroll with it instead
  // of stretching the workspace behind it.
  $effect(() => {
    void active;
    scroller?.scrollTo({ top: 0 });
  });
  const personal = [
    {
      id: 'account',
      label: 'Account',
      icon: UserRound,
      keywords:
        'profile picture avatar password display timezone time format workspace poster video density watched',
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
        'health storage cache docker tools updates maintenance window timezone time format diagnostics transcoding video audio',
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
        'seerr recyclarr radarr sonarr lidarr bazarr prowlarr nzbget downloads requests profiles trash guides indexers updates',
    },
    {
      id: 'retention',
      label: 'Auto-delete',
      icon: Hourglass,
      keywords:
        'auto-delete retention watched deletion grace period movies tv seasons videos storage limit pins library files ownership reconcile',
    },
    {
      id: 'backups',
      label: 'Backups',
      icon: ArchiveRestore,
      keywords:
        'restore encryption archives passphrase automatic schedule retention maintenance window',
    },
    {
      id: 'people',
      label: 'People',
      icon: UsersRound,
      keywords: 'users permissions roles password request approval seerr',
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
  // One line under the title explains what each page is for.
  const descriptions: Record<string, string> = {
    account:
      'Your profile, display preferences, appearance and how you sign in.',
    playback:
      'Defaults for new playback sessions. Choices made inside the player override them for that title.',
    online: 'Accounts connected to this server for YouTube, Twitch and Kick.',
    devices: 'Browsers, desktops and media clients that can use your account.',
    mpv: 'The desktop player, its bundled tools and their configuration.',
    connection: 'This desktop app and the server it connects to.',
    server: 'Health, defaults and maintenance for this Thelxinoe server.',
    analysis: 'Finds intro and credit segments so viewers can skip them.',
    providers:
      'Server-wide applications that let people connect their own YouTube, Twitch and Kick accounts.',
    services:
      'Managed containers and connections for requests, downloads and library automation.',
    retention:
      'Free up space by removing media once the right people have watched it.',
    backups:
      'Encrypted archives of server state, credentials, deployment information and managed-service appdata.',
    people: 'Accounts on this server and what they can do.',
    jobs: 'Background jobs running on this server.',
    audit:
      'History of administrative actions and media deletions on this server.',
  };
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
  const description = $derived(descriptions[active] ?? '');
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
  class="settings-layout mx-auto flex h-(--workspace-height) min-h-0 w-full max-w-520 items-start gap-8 px-8 pt-7 narrow:gap-5 narrow:px-5 compact:h-auto compact:min-h-full compact:flex-col compact:gap-6 compact:px-4 compact:py-4"
>
  <nav
    aria-label="Settings navigation"
    data-sidebar-resize="xy-pos"
    class="settings-navigation @container/settings-nav max-h-full w-49 shrink-0 space-y-6 overflow-y-auto py-1 pb-7 compact:max-h-none compact:w-full compact:space-y-4 compact:overflow-visible compact:pb-0"
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
                  $activity.filter((entry) => entry.target === item.id),
                )}
                active={active === item.id}
                href={routeHref(sectionRoute('Settings', item.id))}
                onclick={(event) => {
                  if (!followLink(event)) return;
                  event.preventDefault();
                  openSettings(item.id);
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
                  activity={$activity.filter(
                    (entry) => entry.target === item.id,
                  )}
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
    class="settings-content @container/settings flex min-h-0 min-w-0 flex-1 flex-col self-stretch compact:w-full compact:self-auto"
    data-page-region="settings"
    data-sidebar-resize="xy-pos"
    data-sidebar-resize-origin
  >
    <header
      class="settings-header mb-6 min-w-0 shrink-0 compact:mb-5"
      data-sidebar-resize="xy-pos"
    >
      <h1 class="text-[25px] font-semibold tracking-tight">{title}</h1>
      {#if description}
        <p class="mt-1.5 mb-0 text-[13px] leading-relaxed text-muted">
          {description}
        </p>
      {/if}
    </header>
    {#if subheader}<div class="shrink-0">{@render subheader()}</div>{/if}
    <div
      bind:this={scroller}
      class="settings-scroll relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto pb-7 [scrollbar-gutter:stable] compact:overflow-visible compact:pb-0"
    >
      <div class="settings-panels settings-grid">
        {@render children()}
      </div>
    </div>
  </div>
</div>
