<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { Globe, Laptop, SearchX, Smartphone } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  import DesktopVersion from '../DesktopVersion.svelte';
  import { prefetchSettings, settingsPages } from '../pages';
  import { api, desktop, serverUrl, type User, type Job } from '../api';
  import { inBackground } from '../page-readiness';
  import { captureSession } from '../session';
  import { withVerification } from '../authentication';
  import { LatestRequest } from '../latest-request';
  import type { AppRoute, SettingsSection } from '../navigation';
  import { isServerUpdateInterruption } from '../server-updates';
  import type { ApprovalUser } from '../services/feature';
  import SettingsLayout from '../ui/SettingsLayout.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import SettingRow from '../ui/SettingRow.svelte';
  import SettingsAnchors from '../ui/SettingsAnchors.svelte';
  import StatusChip from '../ui/StatusChip.svelte';
  import Notice from '../ui/Notice.svelte';
  import FormField from '../ui/FormField.svelte';
  import {
    cardBlockClass,
    emptyClass,
    formControlClass,
    statGridClass,
    statTileClass,
  } from '../ui/styles';
  import Button from '../ui/Button.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import Skeleton from '../ui/Skeleton.svelte';
  import { Accordion } from 'bits-ui';
  import { providers } from '../providers/availability';
  import { settingsGroups } from './sections';
  import { searchSettings } from './search';
  import SettingsSearchGroup from './SettingsSearchGroup.svelte';
  let {
    user,
    route,
    timeFormat,
    preferencesRevision,
    accountRevision,
    revision,
    catalogRevision = 0,
    scans = {},
    navigate,
    openSettings,
    sessionEnded,
    switchServer,
    displayChanged,
    profileChanged,
  } = $props<{
    user: User;
    route: AppRoute;
    timeFormat: '12h' | '24h';
    preferencesRevision: number;
    accountRevision: number;
    revision: number;
    catalogRevision?: number;
    scans?: Record<string, { completed: number; total: number }>;
    navigate: (name: string) => void;
    openSettings: (name: string, service?: string) => void;
    sessionEnded: () => void;
    switchServer: (address: string) => Promise<void>;
    displayChanged: (timezone: string, format: '12h' | '24h') => void;
    profileChanged: (id: string, avatar: string | null) => void;
  }>();
  const settingsSection = $derived(route.settings);
  const ids = $props.id();
  type Session = {
    id: string;
    name: string;
    transport: string;
    last_seen: number;
    remembered?: boolean;
  };
  let sessions = $state<Session[]>([]),
    users = $state<User[]>([]),
    jobs = $state<Job[]>([]),
    currentSession = $state('');
  let newUsername = $state(''),
    newPassword = $state(''),
    newRole = $state<'admin' | 'user'>('user'),
    expandedUser = $state('');
  // Who may request without review, keyed by user id. Only people the
  // server lists get a toggle.
  let approvals = $state<Record<string, boolean>>({}),
    approvalError = $state('');
  let timezone = $state('UTC'),
    serverAddress = $state(untrack(serverUrl));
  let health = $state<{
    version: string;
    controller: boolean;
    cache_free_bytes: number;
  } | null>(null);
  let busy = $state(false),
    error = $state('');
  const requests = new LatestRequest();
  let query = $state(''),
    term = $state('');
  // Search once typing pauses so each keystroke does not mount and load pages.
  $effect(() => {
    const next = query.trim();
    if (next.length < 2) {
      term = '';
      return;
    }
    const timer = setTimeout(() => (term = next), 150);
    return () => clearTimeout(timer);
  });
  // Opening another page, including from a link inside a result, ends the search.
  $effect(() => {
    void settingsSection;
    untrack(() => (query = ''));
  });
  const groups = $derived(settingsGroups(user, $providers));
  const results = $derived(
    term
      ? searchSettings(
          term,
          groups.flatMap((group) => group.items),
        )
      : undefined,
  );
  // The services page links from search results instead of rendering in them.
  const shownSections = $derived<SettingsSection[]>(
    results
      ? results
          .filter((result) => !result.links)
          .map((result) => result.page.id)
      : [settingsSection],
  );
  const shownKey = $derived(shownSections.join(' '));
  let loadingSections = $state<SettingsSection[]>([]);
  const loadedSections = $state<Record<string, boolean>>({});
  const loadingData = (section: SettingsSection) =>
    loadingSections.includes(section) && !loadedSections[section];
  const ownsSession = captureSession();
  const serverAnchors = [
    { id: 'server-status', label: 'Status' },
    { id: 'server-library', label: 'Library' },
    { id: 'server-display', label: 'Display' },
    { id: 'server-updates', label: 'Updates' },
    { id: 'server-transcoding', label: 'Transcoding' },
    { id: 'server-tools', label: 'Tools' },
    { id: 'server-identity', label: 'Identity provider' },
    { id: 'server-overview', label: 'Overview' },
    { id: 'server-devices', label: 'Devices' },
  ];
  const transportIcon = (transport: string) =>
    transport === 'desktop'
      ? Laptop
      : transport === 'mobile'
        ? Smartphone
        : Globe;
  const seen = (session: Session) =>
    new Date(session.last_seen * 1000).toLocaleString(undefined, {
      timeZone: user.timezone,
      hour12: timeFormat === '12h',
    });
  onDestroy(() => requests.invalidate());
  onMount(() => {
    const prefetch = new AbortController();
    void prefetchSettings(
      groups.flatMap((group) => group.items.map((item) => item.id)),
      prefetch.signal,
    );
    return () => prefetch.abort();
  });
  $effect(() => {
    void shownKey;
    void revision;
    untrack(() => void loadSettings());
  });
  async function act(fn: () => Promise<void>) {
    const ownsSession = captureSession();
    busy = true;
    error = '';
    try {
      await fn();
    } catch (caught) {
      if (ownsSession()) error = String(caught);
    } finally {
      if (ownsSession()) busy = false;
    }
  }
  async function loadSettings() {
    const current = requests.begin();
    const sections = shownSections;
    loadingSections = sections;
    error = '';
    try {
      await Promise.all(
        sections.map((section) => loadSection(section, current)),
      );
    } catch (caught) {
      if (current() && !isServerUpdateInterruption(caught))
        error = String(caught);
    } finally {
      if (current()) {
        for (const section of sections) loadedSections[section] = true;
        loadingSections = [];
      }
    }
  }
  async function loadSection(section: SettingsSection, current: () => boolean) {
    if (section === 'devices') {
      const value = await api<{ items: Session[]; current: string }>(
        '/auth/sessions',
      );
      if (current()) {
        sessions = value.items;
        currentSession = value.current;
      }
    } else if (section === 'people') {
      const [people, approval] = await Promise.all([
        api<{ items: User[] }>('/users'),
        api<{ items: ApprovalUser[] }>('/admin/acquisition/users').then(
          (value) => ({ items: value.items, error: '' }),
          (caught: unknown) => ({
            items: [] as ApprovalUser[],
            error: String(caught),
          }),
        ),
      ]);
      if (current()) {
        users = people.items;
        approvals = Object.fromEntries(
          approval.items.map((entry) => [entry.id, entry.enabled]),
        );
        approvalError = approval.error;
      }
    } else if (section === 'jobs') {
      const value = await api<{ items: Job[] }>('/admin/jobs');
      if (current()) jobs = value.items;
    } else if (section === 'server') {
      // Health waits on the Docker controller; its tiles show their own loading.
      const value = await inBackground(() =>
        api<NonNullable<typeof health>>('/admin/health'),
      );
      if (current()) health = value;
    } else if (section === 'backups') {
      const value = await api<{ timezone: string }>('/admin/settings');
      if (current()) timezone = value.timezone;
    }
  }
  async function createUser() {
    await act(async () => {
      await withVerification(async () => {
        await api('/users', 'POST', {
          username: newUsername,
          password: newPassword,
          role: newRole,
        });
        if (!ownsSession()) return;
        newUsername = newPassword = '';
        await loadSettings();
      });
    });
  }
</script>

{#snippet loading(label: string)}
  <ContentSkeleton {label} variant="settings" class="col-span-full" />
{/snippet}
{#snippet failed(caught: unknown)}
  <Notice variant="error" role="alert" class="mb-0">{String(caught)}</Notice>
{/snippet}
{#snippet serverSubheader()}
  <SettingsAnchors items={serverAnchors} />
{/snippet}

{#snippet content(section: SettingsSection)}
  {#if section === 'account'}
    {#await settingsPages.UserPreferences()}{@render loading(
        'Loading settings',
      )}{:then { default: UserPreferences }}<UserPreferences
        {user}
        revision={preferencesRevision}
        avatarChanged={(id, avatar) => {
          profileChanged(id, avatar);
        }}
        changed={(zone, format) => {
          displayChanged(zone, format);
        }}
      />{:catch caught}{@render failed(caught)}{/await}
    {#await settingsPages.AuthSettings()}{@render loading(
        'Loading settings',
      )}{:then { default: AuthSettings }}<AuthSettings
      />{:catch caught}{@render failed(caught)}{/await}{/if}
  {#if section === 'online'}{#await settingsPages.OnlineAccounts()}{@render loading(
        'Loading settings',
      )}{:then { default: OnlineAccounts }}<OnlineAccounts
        revision={accountRevision}
        navigate={(name) => void navigate(name)}
        configureProviders={user.role === 'admin'
          ? () => openSettings('providers')
          : undefined}
      />{:catch caught}{@render failed(caught)}{/await}{/if}
  {#if section === 'playback'}{#await settingsPages.PlaybackSettings()}{@render loading(
        'Loading settings',
      )}{:then { default: PlaybackSettings }}<PlaybackSettings
      />{:catch caught}{@render failed(caught)}{/await}
    {#await settingsPages.SegmentSettings()}{@render loading(
        'Loading settings',
      )}{:then { default: SegmentSettings }}<SegmentSettings
      />{:catch caught}{@render failed(caught)}{/await}
  {/if}
  {#if section === 'devices'}
    <SettingsCard
      title="Your devices"
      description="Browsers and desktops signed in to your account. Revoking this device signs you out."
    >
      {#if loadingData('devices')}<div class={cardBlockClass}>
          <ContentSkeleton label="Loading devices" />
        </div>{/if}
      {#each sessions as session (session.id)}
        {@const Icon = transportIcon(session.transport)}
        <SettingRow
          label={session.name}
          description={`${session.transport} · ${seen(session)}`}
        >
          {#snippet lead()}
            <Icon size={16} class="shrink-0 text-muted" aria-hidden="true" />
          {/snippet}
          {#snippet badge()}
            {#if session.id === currentSession}<StatusChip
                tone="info"
                dot={false}>This device</StatusChip
              >{/if}
            {#if session.remembered}<StatusChip dot={false}
                >Remembered</StatusChip
              >{/if}
          {/snippet}
          <Button
            size="form"
            variant="ghost"
            disabled={busy}
            onclick={() =>
              act(async () => {
                const revoked = await withVerification(async () => {
                  await api(`/auth/sessions/${session.id}`, 'DELETE');
                });
                if (!revoked || !ownsSession()) return;
                if (session.id === currentSession) {
                  sessionEnded();
                } else await loadSettings();
              })}>Revoke</Button
          >
        </SettingRow>
      {/each}
    </SettingsCard>
    {#await settingsPages.QuickConnect()}{@render loading(
        'Loading settings',
      )}{:then { default: QuickConnect }}<QuickConnect
        username={user.username}
      />{:catch caught}{@render failed(caught)}{/await}
    {#await settingsPages.AuthDevices()}{@render loading(
        'Loading settings',
      )}{:then { default: AuthDevices }}<AuthDevices
      />{:catch caught}{@render failed(caught)}{/await}
  {/if}
  {#if desktop && section === 'mpv'}
    <SettingsCard
      title="Player and tools"
      description="Install, update and configure the desktop player and its helper tools."
      padded
    >
      {#await settingsPages.MpvSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: MpvSettings }}<MpvSettings
        />{:catch caught}{@render failed(caught)}{/await}
    </SettingsCard>
  {/if}
  {#if desktop && section === 'connection'}
    <SettingsCard
      title="Desktop"
      description="The installed desktop build, how it updates and which server it talks to."
    >
      <SettingRow
        label="Desktop version"
        description="Use the status button to check for updates or install one."
      >
        <DesktopVersion inline />
      </SettingRow>
      {#await settingsPages.DesktopUpdatePreferences()}<div
          class={cardBlockClass}
        >
          <ContentSkeleton
            label="Loading settings"
            variant="settings"
            count={1}
          />
        </div>{:then { default: DesktopUpdatePreferences }}
        <SettingRow
          label="Desktop updates"
          description="Notify shows an update when one is ready. Automatic installs it and restarts once playback stops."
        >
          <DesktopUpdatePreferences />
        </SettingRow>
      {:catch caught}<div class={cardBlockClass}>
          {@render failed(caught)}
        </div>{/await}
      <SettingRow
        label="Server address"
        description="Switching servers signs you out of this one."
        for={`${ids}-server`}
      >
        <form
          class="flex flex-wrap items-center gap-2"
          onsubmit={(e) => {
            e.preventDefault();
            void act(async () => {
              await switchServer(serverAddress);
            });
          }}
        >
          <input
            id={`${ids}-server`}
            class={twMerge(formControlClass, 'w-72 compact:w-full')}
            bind:value={serverAddress}
            required
          />
          <Button size="form" variant="secondary" type="submit" disabled={busy}
            >Change server</Button
          >
        </form>
      </SettingRow>
    </SettingsCard>
  {/if}
  {#if user.role === 'admin'}
    {#if section === 'analysis'}{#await settingsPages.SegmentSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: SegmentSettings }}<SegmentSettings
          admin
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'backups'}{#await settingsPages.BackupSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: BackupSettings }}<BackupSettings
          {timezone}
          {timeFormat}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'providers'}{#await settingsPages.OnlineSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: OnlineSettings }}<OnlineSettings
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'services'}{#await settingsPages.ServicesSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: ServicesSettings }}<ServicesSettings
          {timeFormat}
          selected={route.service}
          workflow={route.workflow}
          onSelect={(service) => openSettings('services', service)}
          openSettings={(name) => openSettings(name)}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'retention'}{#await settingsPages.RetentionSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: RetentionSettings }}<RetentionSettings
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'retention'}{#await settingsPages.ManagerOwnership()}{@render loading(
          'Loading settings',
        )}{:then { default: ManagerOwnership }}<ManagerOwnership
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'server'}
      <SettingsCard
        id="server-status"
        aria-label="Server status"
        title="Server status"
        description="Installed version, update availability and controller health."
      >
        <div class={statGridClass}>
          <div class={statTileClass}>
            {#await settingsPages.ProductVersion()}<Skeleton
                class="h-6 w-24"
              />{:then { default: ProductVersion }}<ProductVersion
                installed={health?.version ?? '—'}
              />{:catch caught}{@render failed(caught)}{/await}
          </div>
          <div class={statTileClass}>
            {#if loadingData('server')}<Skeleton
                class="h-6 w-24"
              />{:else}<strong
                >{health?.controller ? 'Connected' : 'Unavailable'}</strong
              >{/if}
            <small>Docker controller</small>
          </div>
          <div class={statTileClass}>
            {#if loadingData('server')}<Skeleton
                class="h-6 w-24"
              />{:else}<strong
                >{health
                  ? `${(health.cache_free_bytes / 1024 ** 3).toFixed(1)} GB`
                  : '—'}</strong
              >{/if}
            <small>Cache space available</small>
          </div>
        </div>
      </SettingsCard>
      {#await settingsPages.LibraryFolders()}{@render loading(
          'Loading settings',
        )}{:then { default: LibraryFolders }}<LibraryFolders
          timezone={user.timezone}
          {timeFormat}
          revision={catalogRevision}
          {scans}
        />{:catch caught}{@render failed(caught)}{/await}
      {#await settingsPages.ServerDisplayDefaults()}{@render loading(
          'Loading settings',
        )}{:then { default: ServerDisplayDefaults }}<ServerDisplayDefaults
          revision={preferencesRevision}
          changed={(value) => (timezone = value)}
        />{:catch caught}{@render failed(caught)}{/await}
      <SettingsCard
        id="server-updates"
        title="Updates and maintenance"
        description="How server releases are installed and when scheduled work runs."
        autoSave
      >
        {#await settingsPages.ProductUpdatePreferences()}<div
            class={cardBlockClass}
          >
            <ContentSkeleton
              label="Loading settings"
              variant="settings"
              count={2}
            />
          </div>{:then { default: ProductUpdatePreferences }}<ProductUpdatePreferences
          />{:catch caught}<div class={cardBlockClass}>
            {@render failed(caught)}
          </div>{/await}
      </SettingsCard>
      {#await settingsPages.ServerTranscodingSettings()}{@render loading(
          'Loading transcoding settings',
        )}{:then { default: ServerTranscodingSettings }}<ServerTranscodingSettings
          revision={preferencesRevision}
        />{:catch caught}{@render failed(caught)}{/await}
      {#await settingsPages.ServerTools()}{@render loading(
          'Loading settings',
        )}{:then { default: ServerTools }}<ServerTools
        />{:catch caught}{@render failed(caught)}{/await}
      {#await settingsPages.OidcSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: OidcSettings }}<OidcSettings
        />{:catch caught}{@render failed(caught)}{/await}
      {#await settingsPages.AdminOperations()}{@render loading(
          'Loading settings',
        )}{:then { default: AdminOperations }}<AdminOperations
          {timezone}
          {timeFormat}
        />{:catch caught}{@render failed(caught)}{/await}
    {/if}
    {#if section === 'people'}
      <SettingsCard
        title="Add user"
        description="New people sign in with a username and password. They can add passkeys or link an identity provider later."
        padded
      >
        <form
          class="flex flex-wrap items-end gap-3 compact:flex-col compact:items-stretch [&_label]:m-0 [&_label]:min-w-[min(180px,100%)] [&_label]:flex-1 compact:[&_label]:w-full"
          aria-label="Create user"
          onsubmit={(e) => {
            e.preventDefault();
            void createUser();
          }}
        >
          <FormField
            >Username<input
              class={formControlClass}
              bind:value={newUsername}
              required
              autocomplete="off"
            /></FormField
          ><FormField
            >Password<input
              class={formControlClass}
              bind:value={newPassword}
              type="password"
              required
              minlength="8"
              autocomplete="new-password"
            /></FormField
          ><FormField
            >Role<select class={formControlClass} bind:value={newRole}
              ><option value="user">User</option><option value="admin"
                >Administrator</option
              ></select
            ></FormField
          ><Button size="form" type="submit" disabled={busy}>Add user</Button>
        </form>
      </SettingsCard>
      <SettingsCard
        title="User access"
        description="Roles, automatic request approval, recovery links and account removal for everyone on this server."
      >
        {#if loadingData('people')}<div class={cardBlockClass}>
            <ContentSkeleton label="Loading users" />
          </div>{/if}
        {#if approvalError}<div class={cardBlockClass}>
            <Notice
              variant="error"
              role="alert"
              aria-label="Request approval settings load error"
              class="mb-0"
              >Failed to load request approval settings. {approvalError}</Notice
            >
          </div>{/if}
        <Accordion.Root
          type="single"
          bind:value={expandedUser}
          class="divide-y divide-line"
        >
          {#each users as person (person.id)}{#await settingsPages.UserAdministration()}<div
                class={cardBlockClass}
              >
                <ContentSkeleton label="Loading settings" count={1} />
              </div>{:then { default: UserAdministration }}<UserAdministration
                {person}
                approval={approvals[person.id]}
                currentId={user.id}
                changed={loadSettings}
                close={() => (expandedUser = '')}
              />{:catch caught}<div class={cardBlockClass}>
                {@render failed(caught)}
              </div>{/await}{/each}
        </Accordion.Root>
      </SettingsCard>
    {/if}
    {#if section === 'audit'}{#await settingsPages.AuditSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: AuditSettings }}<AuditSettings
          {user}
          {timeFormat}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'jobs'}{#await settingsPages.ActivitySettings()}{@render loading(
          'Loading settings',
        )}{:then { default: ActivitySettings }}<ActivitySettings
          {jobs}
          loading={loadingData('jobs')}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
  {/if}
{/snippet}

<SettingsLayout
  {groups}
  active={settingsSection}
  bind:query
  {results}
  {openSettings}
  subheader={!results && settingsSection === 'server' && user.role === 'admin'
    ? serverSubheader
    : undefined}
>
  {#if error}<Notice variant="error" role="alert" class="mb-0">{error}</Notice
    >{/if}
  {#if results}
    {#each results as result (result.page.id)}
      <SettingsSearchGroup
        {result}
        open={(section, service) => {
          query = '';
          openSettings(section, service);
        }}>{@render content(result.page.id)}</SettingsSearchGroup
      >
    {:else}
      <div role="status" class={[emptyClass, 'col-span-full']}>
        <SearchX size={28} strokeWidth={1.6} aria-hidden="true" />
        <h2 class="m-0 text-[15px] font-semibold">No matching settings</h2>
        <p class="mt-1.5 mb-0">
          Nothing matches “{term}”. Try a page name or a setting label.
        </p>
      </div>
    {/each}
  {:else}
    {@render content(settingsSection)}
  {/if}
</SettingsLayout>
