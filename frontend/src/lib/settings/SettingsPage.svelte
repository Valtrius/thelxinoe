<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { SearchX } from '@lucide/svelte';
  import { twMerge } from 'tailwind-merge';
  import DesktopVersion from '../DesktopVersion.svelte';
  import { prefetchSettings, settingsPages } from '../pages';
  import { desktop, serverUrl, type User } from '../api';
  import { captureSession } from '../session';
  import type { AppRoute, SettingsSection } from '../navigation';
  import SettingsLayout from '../ui/SettingsLayout.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import SettingRow from '../ui/SettingRow.svelte';
  import SettingsAnchors from '../ui/SettingsAnchors.svelte';
  import Notice from '../ui/Notice.svelte';
  import { cardBlockClass, emptyClass, formControlClass } from '../ui/styles';
  import Button from '../ui/Button.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import { providers } from '../providers/availability';
  import { settingsGroups } from './sections';
  import { searchSettings } from './search';
  import SettingsSearchGroup from './SettingsSearchGroup.svelte';
  import DeviceSessions from './DeviceSessions.svelte';
  import PeopleSettings from './PeopleSettings.svelte';
  import ServerStatus from './ServerStatus.svelte';
  let {
    user,
    route,
    navigate,
    openSettings,
    sessionEnded,
    switchServer,
    profileChanged,
  } = $props<{
    user: User;
    route: AppRoute;
    navigate: (name: string) => void;
    openSettings: (name: string, service?: string) => void;
    sessionEnded: () => void;
    switchServer: (address: string) => Promise<void>;
    profileChanged: (id: string, avatar: string | null) => void;
  }>();
  const settingsSection = $derived(route.settings);
  const ids = $props.id();
  let serverAddress = $state(untrack(serverUrl));
  let busy = $state(false),
    error = $state('');
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
  const serverAnchors = [
    { id: 'server-status', label: 'Status' },
    { id: 'server-display', label: 'Display' },
    { id: 'server-updates', label: 'Updates' },
    { id: 'server-transcoding', label: 'Transcoding' },
    { id: 'server-tools', label: 'Tools' },
    { id: 'server-identity', label: 'Identity provider' },
    { id: 'server-overview', label: 'Overview' },
    { id: 'server-devices', label: 'Devices' },
  ];
  onMount(() => {
    const prefetch = new AbortController();
    void prefetchSettings(
      groups.flatMap((group) => group.items.map((item) => item.id)),
      prefetch.signal,
    );
    return () => prefetch.abort();
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
        avatarChanged={(id, avatar) => {
          profileChanged(id, avatar);
        }}
      />{:catch caught}{@render failed(caught)}{/await}
    {#await settingsPages.AuthSettings()}{@render loading(
        'Loading settings',
      )}{:then { default: AuthSettings }}<AuthSettings
      />{:catch caught}{@render failed(caught)}{/await}{/if}
  {#if section === 'online'}{#await settingsPages.OnlineAccounts()}{@render loading(
        'Loading settings',
      )}{:then { default: OnlineAccounts }}<OnlineAccounts
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
    <DeviceSessions {sessionEnded} />
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
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'providers'}{#await settingsPages.OnlineSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: OnlineSettings }}<OnlineSettings
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'services'}{#await settingsPages.ServicesSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: ServicesSettings }}<ServicesSettings
          selected={route.service}
          workflow={route.workflow}
          onSelect={(service) => openSettings('services', service)}
          openSettings={(name) => openSettings(name)}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'jellyfin-import'}{#await settingsPages.JellyfinImport()}{@render loading(
          'Loading settings',
        )}{:then { default: JellyfinImport }}<JellyfinImport
          openSettings={(name, service) => openSettings(name, service)}
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
      <ServerStatus />
      {#await settingsPages.ServerDisplayDefaults()}{@render loading(
          'Loading settings',
        )}{:then { default: ServerDisplayDefaults }}<ServerDisplayDefaults
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
        />{:catch caught}{@render failed(caught)}{/await}
    {/if}
    {#if section === 'people'}<PeopleSettings {user} />{/if}
    {#if section === 'audit'}{#await settingsPages.AuditSettings()}{@render loading(
          'Loading settings',
        )}{:then { default: AuditSettings }}<AuditSettings
          {user}
        />{:catch caught}{@render failed(caught)}{/await}{/if}
    {#if section === 'jobs'}{#await settingsPages.ActivitySettings()}{@render loading(
          'Loading settings',
        )}{:then { default: ActivitySettings }}<ActivitySettings
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
