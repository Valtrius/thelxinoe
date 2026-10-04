<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import DesktopVersion from '../DesktopVersion.svelte';
  import { settingsPages } from '../pages';
  import { api, desktop, serverUrl, type User, type Job } from '../api';
  import { captureSession } from '../session';
  import { withVerification } from '../authentication';
  import { LatestRequest } from '../latest-request';
  import type { AppRoute } from '../navigation';
  import { isServerUpdateInterruption } from '../server-updates';
  import SettingsLayout from '../ui/SettingsLayout.svelte';
  import Notice from '../ui/Notice.svelte';
  import FormField from '../ui/FormField.svelte';
  import {
    formControlClass,
    inlineFormClass,
    rowClass,
    statsClass,
  } from '../ui/styles';
  import Button from '../ui/Button.svelte';
  import Panel from '../ui/Panel.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import Skeleton from '../ui/Skeleton.svelte';
  import { Accordion } from 'bits-ui';
  let {
    user,
    route,
    timeFormat,
    preferencesRevision,
    accountRevision,
    revision,
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
    navigate: (name: string) => void;
    openSettings: (name: string, service?: string) => void;
    sessionEnded: () => void;
    switchServer: (address: string) => Promise<void>;
    displayChanged: (timezone: string, format: '12h' | '24h') => void;
    profileChanged: (id: string, avatar: string | null) => void;
  }>();
  const settingsSection = $derived(route.settings);
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
  let loadingSection = $state<string | null>(null);
  const loadedSections = $state<Record<string, boolean>>({});
  const loadingData = $derived(
    loadingSection === settingsSection && !loadedSections[settingsSection],
  );
  const ownsSession = captureSession();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void settingsSection;
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
    const section = settingsSection;
    loadingSection = section;
    error = '';
    try {
      if (settingsSection === 'devices') {
        const value = await api<{ items: Session[]; current: string }>(
          '/auth/sessions',
        );
        if (current()) {
          sessions = value.items;
          currentSession = value.current;
        }
      } else if (settingsSection === 'people') {
        const value = await api<{ items: User[] }>('/users');
        if (current()) users = value.items;
      } else if (settingsSection === 'jobs') {
        const value = await api<{ items: Job[] }>('/admin/jobs');
        if (current()) jobs = value.items;
      } else if (settingsSection === 'server') {
        const value = await api<NonNullable<typeof health>>('/admin/health');
        if (current()) health = value;
      } else if (settingsSection === 'backups') {
        const value = await api<{ timezone: string }>('/admin/settings');
        if (current()) timezone = value.timezone;
      }
    } catch (caught) {
      if (current() && !isServerUpdateInterruption(caught))
        error = String(caught);
    } finally {
      if (current()) {
        loadedSections[section] = true;
        loadingSection = null;
      }
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

<SettingsLayout {user} active={settingsSection} {openSettings}>
  {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
  {#if settingsSection === 'account'}
    {#await settingsPages.UserPreferences()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: UserPreferences }}<UserPreferences
        {user}
        revision={preferencesRevision}
        avatarChanged={(id, avatar) => {
          profileChanged(id, avatar);
        }}
        changed={(zone, format) => {
          displayChanged(zone, format);
        }}
      />{:catch error}<Notice variant="error" role="alert"
        >{String(error)}</Notice
      >{/await}
    {#await settingsPages.AuthSettings()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: AuthSettings }}<AuthSettings />{:catch error}<Notice
        variant="error"
        role="alert">{String(error)}</Notice
      >{/await}{/if}
  {#if settingsSection === 'online'}{#await settingsPages.OnlineAccounts()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: OnlineAccounts }}<OnlineAccounts
        revision={accountRevision}
        navigate={(name) => void navigate(name)}
        configureProviders={user.role === 'admin'
          ? () => openSettings('providers')
          : undefined}
      />{:catch error}<Notice variant="error" role="alert"
        >{String(error)}</Notice
      >{/await}{/if}
  {#if settingsSection === 'playback'}{#await settingsPages.PlaybackSettings()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: PlaybackSettings }}<PlaybackSettings
      />{:catch error}<Notice variant="error" role="alert"
        >{String(error)}</Notice
      >{/await}
    {#await settingsPages.SegmentSettings()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: SegmentSettings }}<SegmentSettings
      />{:catch error}<Notice variant="error" role="alert"
        >{String(error)}</Notice
      >{/await}
  {/if}
  {#if settingsSection === 'devices'}{#await settingsPages.AuthDevices()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: AuthDevices }}<AuthDevices />{:catch caught}<Notice
        variant="error"
        role="alert">{String(caught)}</Notice
      >{/await}{#await settingsPages.QuickConnect()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: QuickConnect }}<QuickConnect
        username={user.username}
      />{:catch error}<Notice variant="error" role="alert"
        >{String(error)}</Notice
      >{/await}{/if}
  {#if desktop && settingsSection === 'mpv'}{#await settingsPages.MpvSettings()}<ContentSkeleton
        label="Loading settings"
        variant="settings"
        class="col-span-full"
      />{:then { default: MpvSettings }}<MpvSettings />{:catch error}<Notice
        variant="error"
        role="alert">{String(error)}</Notice
      >{/await}{/if}
  {#if desktop && settingsSection === 'connection'}<Panel>
      <h2>Desktop</h2>
      <div class={statsClass}>
        <DesktopVersion />
      </div>
      {#await settingsPages.DesktopUpdatePreferences()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: DesktopUpdatePreferences }}<DesktopUpdatePreferences
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}
      <form
        class={inlineFormClass}
        onsubmit={(e) => {
          e.preventDefault();
          void act(async () => {
            await switchServer(serverAddress);
          });
        }}
      >
        <FormField
          >Server address<input
            class={formControlClass}
            bind:value={serverAddress}
            required
          /></FormField
        ><Button size="form" variant="secondary" type="submit" disabled={busy}
          >Change server</Button
        >
      </form>
    </Panel>{/if}
  {#if settingsSection === 'devices'}<Panel>
      <h2>Your devices</h2>
      <p class="text-muted">
        Revoke access to a browser or desktop at any time.
      </p>
      {#if loadingData}<ContentSkeleton label="Loading devices" />{/if}
      {#each sessions as session (session.id)}<div class={rowClass}>
          <div>
            <strong>{session.name}</strong><small
              >{session.transport} · {new Date(
                session.last_seen * 1000,
              ).toLocaleString(undefined, {
                timeZone: user.timezone,
                hour12: timeFormat === '12h',
              })}{session.id === currentSession ? ' · This device' : ''}</small
            >
          </div>
          <Button
            size="form"
            variant="secondary"
            type="submit"
            disabled={busy}
            onclick={() =>
              act(async () => {
                await withVerification(async () => {
                  await api(`/auth/sessions/${session.id}`, 'DELETE');
                });
                if (!ownsSession()) return;
                if (session.id === currentSession) {
                  sessionEnded();
                } else await loadSettings();
              })}>Revoke</Button
          >
        </div>{/each}
    </Panel>
  {/if}
  {#if user.role === 'admin'}
    {#if settingsSection === 'analysis'}{#await settingsPages.SegmentSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: SegmentSettings }}<SegmentSettings
          admin
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'backups'}{#await settingsPages.BackupSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: BackupSettings }}<BackupSettings
          {timezone}
          {timeFormat}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'providers'}{#await settingsPages.OnlineSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: OnlineSettings }}<OnlineSettings
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'services'}{#await settingsPages.ServicesSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: ServicesSettings }}<ServicesSettings
          {timeFormat}
          selected={route.service}
          workflow={route.workflow}
          onSelect={(service) => openSettings('services', service)}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'services'}{#await settingsPages.ManagerOwnership()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: ManagerOwnership }}<ManagerOwnership
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'retention'}{#await settingsPages.RetentionSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: RetentionSettings }}<RetentionSettings
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'server'}<Panel>
        <div class="flex items-center justify-between gap-3">
          <h2>Server status</h2>
        </div>
        <div
          class="grid divide-y divide-line [&>div]:py-3 [&_strong]:text-base [&_strong]:font-medium [&_small]:mt-1 [&_small]:block [&_small]:text-xs [&_small]:text-muted"
        >
          {#await settingsPages.ProductVersion()}<ContentSkeleton
              label="Loading settings"
              variant="settings"
              class="col-span-full"
            />{:then { default: ProductVersion }}<ProductVersion
              installed={health?.version ?? '—'}
            />{:catch error}<Notice variant="error" role="alert"
              >{String(error)}</Notice
            >{/await}
          <div>
            {#if loadingData}<Skeleton class="h-5 w-24" />{:else}<strong
                >{health?.controller ? 'Connected' : 'Unavailable'}</strong
              >{/if}
            <small>Docker controller</small>
          </div>
          <div>
            {#if loadingData}<Skeleton class="h-5 w-24" />{:else}<strong
                >{health
                  ? `${(health.cache_free_bytes / 1024 ** 3).toFixed(1)} GB`
                  : '—'}</strong
              >{/if}
            <small>Cache space available</small>
          </div>
        </div>
      </Panel>
      {#await settingsPages.ServerDisplayDefaults()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: ServerDisplayDefaults }}<ServerDisplayDefaults
          revision={preferencesRevision}
          changed={(value) => (timezone = value)}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}
      <Panel
        ><h2>Server updates</h2>
        {#await settingsPages.ProductUpdatePreferences()}<ContentSkeleton
            label="Loading settings"
            variant="settings"
            class="col-span-full"
          />{:then { default: ProductUpdatePreferences }}<ProductUpdatePreferences
          />{:catch error}<Notice variant="error" role="alert"
            >{String(error)}</Notice
          >{/await}</Panel
      >
      <Panel class="settings-wide"
        >{#await settingsPages.ServerTools()}<ContentSkeleton
            label="Loading settings"
            variant="settings"
            class="col-span-full"
          />{:then { default: ServerTools }}<ServerTools />{:catch error}<Notice
            variant="error"
            role="alert">{String(error)}</Notice
          >{/await}</Panel
      >
      {#await settingsPages.OidcSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: OidcSettings }}<OidcSettings
        />{:catch caught}<Notice variant="error" role="alert"
          >{String(caught)}</Notice
        >{/await}
      {#await settingsPages.AdminOperations()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: AdminOperations }}<AdminOperations
          {timezone}
          {timeFormat}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}
    {/if}
    {#if settingsSection === 'people'}<Panel>
        <h2>Add user</h2>
        <form
          class={inlineFormClass}
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
      </Panel>
      <Panel class="settings-wide"
        ><h2>User access</h2>
        {#if loadingData}<ContentSkeleton label="Loading users" />{/if}
        <Accordion.Root type="single" bind:value={expandedUser}>
          {#each users as person (person.id)}{#await settingsPages.UserAdministration()}<ContentSkeleton
                label="Loading settings"
                variant="settings"
                class="col-span-full"
              />{:then { default: UserAdministration }}<UserAdministration
                {person}
                currentId={user.id}
                changed={loadSettings}
                close={() => (expandedUser = '')}
              />{:catch error}<Notice variant="error" role="alert"
                >{String(error)}</Notice
              >{/await}{/each}
        </Accordion.Root>
      </Panel>
    {/if}
    {#if settingsSection === 'audit'}{#await settingsPages.AuditSettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: AuditSettings }}<AuditSettings
          {user}
          {timeFormat}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
    {#if settingsSection === 'jobs'}{#await settingsPages.ActivitySettings()}<ContentSkeleton
          label="Loading settings"
          variant="settings"
          class="col-span-full"
        />{:then { default: ActivitySettings }}<ActivitySettings
          {jobs}
          loading={loadingData}
          checkpoint={() =>
            act(async () => {
              await api('/admin/jobs', 'POST', {
                key: crypto.randomUUID(),
              });
              await loadSettings();
            })}
        />{:catch error}<Notice variant="error" role="alert"
          >{String(error)}</Notice
        >{/await}{/if}
  {/if}
</SettingsLayout>
