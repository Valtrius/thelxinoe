<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import {
    ArrowRight,
    ChevronDown,
    ChevronUp,
    RefreshCw,
  } from '@lucide/svelte';
  import { api } from './api';
  import { withVerification } from './authentication';
  import { LatestRequest } from './latest-request';
  import { formatClock } from './providers/utils';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import Button from './ui/Button.svelte';
  import Notice from './ui/Notice.svelte';
  import Switch from './ui/Switch.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { display } from './display.svelte';
  import { dateFormat } from './intl';
  import {
    cardBlockClass,
    cardTableClass,
    formControlClass,
    statGridClass,
    statTileClass,
  } from './ui/styles';

  let { openSettings } = $props<{
    openSettings: (name: string, service?: string) => void;
  }>();

  type Counts = {
    watched: number;
    already_watched: number;
    resume: number;
    favorites: number;
  };
  type Missing = {
    account: string;
    title: string;
    year: number | null;
    episodes: number | null;
    watched: number;
    resume: number;
    favorite: number;
    position: number | null;
  };
  type Report = {
    at: number;
    server: string;
    accounts: (Counts & {
      jellyfin_id: string;
      name: string;
      username: string;
      not_found: number;
    })[];
    not_found: Missing[];
    retention: number;
    groups?: {
      key: string;
      jellyfin: string;
      title: string;
      series: boolean;
      items: number;
      accounts: Counts[] | null;
    }[];
  };
  type Source = { connected: boolean; titles: number; identified: number };
  type Overview = {
    server: { url: string; name: string; version: string } | null;
    last: Report | null;
    library: {
      movies: number;
      episodes: number;
      scanned_at: number | null;
      radarr: Source;
      sonarr: Source;
    };
  };
  type Account = {
    id: string;
    name: string;
    administrator: boolean;
    disabled: boolean;
    played: number;
    in_progress: number;
    favorites: number;
    suggested: string | null;
  };

  const ids = $props.id();
  const requests = new LatestRequest();
  let overview = $state<Overview | null>(null),
    error = $state(''),
    busy = $state(false),
    url = $state(''),
    apiKey = $state('');
  let choosing = $state(false),
    accounts = $state<Account[] | null>(null),
    people = $state<{ id: string; username: string }[]>([]),
    mapping = $state<Record<string, string>>({}),
    wanted = $state({ watched: true, resume: true, favorites: true }),
    previewing = $state(false),
    preview = $state<Report | null>(null),
    showMissing = $state(false);
  const excluded = new SvelteSet<string>();

  const step = $derived(
    !overview
      ? 'loading'
      : !overview.server
        ? 'connect'
        : preview
          ? 'preview'
          : overview.last && !choosing
            ? 'done'
            : 'choose',
  );
  const selected = $derived(
    (accounts ?? [])
      .filter((account) => mapping[account.id])
      .map((account) => ({
        jellyfin_id: account.id,
        user_id: mapping[account.id],
      })),
  );
  // Leaving a title-matched group out takes away what it added.
  const previewed = $derived(
    (preview?.accounts ?? []).map((account, index) => {
      const counts = { ...account };
      for (const group of preview?.groups ?? []) {
        const added = group.accounts?.[index];
        if (!added || !excluded.has(group.key)) continue;
        counts.watched -= added.watched;
        counts.already_watched -= added.already_watched;
        counts.resume -= added.resume;
        counts.favorites -= added.favorites;
      }
      return counts;
    }),
  );

  const n = (value: number) => value.toLocaleString();
  const time = (seconds: number) =>
    dateFormat(undefined, {
      timeZone: display.timezone,
      hour12: display.timeFormat === '12h',
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(seconds * 1000);
  const sum = (rows: Counts[], key: keyof Counts) =>
    rows.reduce((total, row) => total + row[key], 0);
  const missingTitle = (row: Missing) =>
    row.episodes === null
      ? row.year
        ? `${row.title} (${row.year})`
        : row.title
      : row.episodes
        ? `${row.title} · ${n(row.episodes)} ${row.episodes === 1 ? 'episode' : 'episodes'}`
        : row.title;
  function missingState(row: Missing) {
    const parts: string[] = [];
    if (row.episodes === null) {
      if (row.watched) parts.push('Watched');
      if (row.resume && row.position !== null)
        parts.push(`Resume at ${formatClock(row.position)}`);
      if (row.favorite) parts.push('Favorite');
    } else {
      if (row.watched) parts.push(`${n(row.watched)} watched`);
      if (row.resume) parts.push(`${n(row.resume)} in progress`);
      if (row.favorite)
        parts.push(
          row.favorite === 1 ? 'Favorite' : `${n(row.favorite)} favorites`,
        );
    }
    return parts.join(' · ');
  }

  const managers = $derived(
    overview
      ? [
          {
            kind: 'radarr',
            label: 'Radarr',
            source: overview.library.radarr,
            identified: 'Movies also match by TMDB and IMDb IDs',
            titles: 'movies',
            missing:
              'Movies whose file changed since Jellyfin played them can only match by title.',
          },
          {
            kind: 'sonarr',
            label: 'Sonarr',
            source: overview.library.sonarr,
            identified: 'Episodes also match by their series IDs',
            titles: 'shows',
            missing:
              'Episodes whose file changed since Jellyfin played them can only match by title.',
          },
        ]
      : [],
  );

  async function load() {
    const current = requests.begin();
    try {
      const value = await api<Overview>('/admin/jellyfin-import');
      if (!current()) return;
      overview = value;
      if (value.server && !value.last) void choose();
    } catch (caught) {
      if (current()) error = String(caught);
    }
  }
  onMount(() => void load());
  onDestroy(() => requests.invalidate());

  // Jellyfin's accounts and counts are read again each time; choices already made stay.
  async function choose() {
    choosing = true;
    error = '';
    try {
      const value = await api<{
        accounts: Account[];
        people: { id: string; username: string }[];
      }>('/admin/jellyfin-import/accounts');
      people = value.people;
      mapping = Object.fromEntries(
        value.accounts.map((account) => [
          account.id,
          mapping[account.id] ?? account.suggested ?? '',
        ]),
      );
      accounts = value.accounts;
    } catch (caught) {
      error = String(caught);
    }
  }
  async function act(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
    } catch (caught) {
      error = String(caught);
    } finally {
      busy = false;
    }
  }
  const request = () => ({ accounts: selected, ...wanted });
</script>

{#snippet missingTable(rows: Missing[], limit: number)}
  <table class={cardTableClass}>
    <thead><tr><th>In Jellyfin</th><th>Account</th><th>State</th></tr></thead>
    <tbody>
      {#each rows.slice(0, limit) as row, index (index)}
        <tr
          ><td>{missingTitle(row)}</td><td>{row.account}</td><td
            >{missingState(row)}</td
          ></tr
        >
      {/each}
    </tbody>
  </table>
{/snippet}

{#snippet counts(
  rows: (Counts & { name: string; username: string; not_found: number })[],
  already: boolean,
)}
  <div class={statGridClass}>
    <div class={statTileClass}>
      <strong>{n(sum(rows, 'watched'))}</strong><small
        >{already ? 'Newly watched' : 'Marked watched'}</small
      >
    </div>
    <div class={statTileClass}>
      <strong>{n(sum(rows, 'resume'))}</strong><small>Resume points</small>
    </div>
    <div class={statTileClass}>
      <strong>{n(sum(rows, 'favorites'))}</strong><small>Favorites</small>
    </div>
    <div class={statTileClass}>
      <strong>{n(rows.reduce((total, row) => total + row.not_found, 0))}</strong
      ><small>Not found</small>
    </div>
  </div>
  <div class="overflow-x-auto">
    <table class={cardTableClass}>
      <thead>
        <tr>
          <th>Jellyfin account</th>
          <th>Into</th>
          <th class="text-right"
            >{already ? 'Newly watched' : 'Marked watched'}</th
          >
          {#if already}<th class="text-right">Already watched</th>{/if}
          <th class="text-right">Resume points</th>
          <th class="text-right">Favorites</th>
          <th class="text-right">Not found</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as row (row.name)}
          <tr>
            <td class="font-[550]">{row.name}</td>
            <td>{row.username}</td>
            <td class="text-right tabular-nums">{n(row.watched)}</td>
            {#if already}<td class="text-right tabular-nums"
                >{n(row.already_watched)}</td
              >{/if}
            <td class="text-right tabular-nums">{n(row.resume)}</td>
            <td class="text-right tabular-nums">{n(row.favorites)}</td>
            <td class="text-right tabular-nums">{n(row.not_found)}</td>
          </tr>
        {/each}
        {#if already}
          {#each (accounts ?? []).filter((account) => !mapping[account.id]) as account (account.id)}
            <tr class="text-muted">
              <td>{account.name}</td>
              <td colspan="6">Skipped</td>
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
{/snippet}

{#if error}<Notice variant="error" role="alert" class="mb-0">{error}</Notice
  >{/if}

{#if step === 'loading'}
  <ContentSkeleton
    label="Loading Jellyfin import"
    variant="settings"
    class="col-span-full"
  />
{:else if overview && step === 'connect'}
  <SettingsCard title="Jellyfin server">
    {#snippet badge()}<StatusChip>Not connected</StatusChip>{/snippet}
    <form
      id={`${ids}-connect`}
      class="divide-y divide-line"
      onsubmit={(event) => {
        event.preventDefault();
        void act(async () => {
          const connected = await withVerification(async () => {
            await api('/admin/jellyfin-import/server', 'PUT', {
              url,
              api_key: apiKey,
            });
          });
          if (!connected) return;
          apiKey = '';
          await load();
        });
      }}
    >
      <SettingRow
        label="Server URL"
        description="The address the Thelxinoe server uses to reach Jellyfin."
        for={`${ids}-url`}
        control="field"
      >
        <input
          id={`${ids}-url`}
          class={formControlClass}
          type="url"
          placeholder="http://jellyfin:8096"
          bind:value={url}
          required
          disabled={busy}
        />
      </SettingRow>
      <SettingRow
        label="API key"
        description="Create one in Jellyfin under Dashboard → API Keys."
        for={`${ids}-key`}
        control="field"
      >
        <input
          id={`${ids}-key`}
          class={formControlClass}
          type="password"
          autocomplete="off"
          bind:value={apiKey}
          required
          disabled={busy}
        />
      </SettingRow>
    </form>
    {#snippet footer()}
      <span></span>
      <Button type="submit" form={`${ids}-connect`} loading={busy}
        >Connect</Button
      >
    {/snippet}
  </SettingsCard>
{:else if overview?.server}
  <SettingsCard
    title="Jellyfin server"
    description={`${overview.server.name} · Jellyfin ${overview.server.version} · ${overview.server.url}`}
  >
    {#snippet badge()}<StatusChip tone="ok">Connected</StatusChip>{/snippet}
    {#snippet actions()}
      <Button
        variant="ghost"
        size="sm"
        disabled={busy || previewing}
        onclick={() =>
          act(async () => {
            await api('/admin/jellyfin-import/server', 'DELETE');
            preview = null;
            accounts = null;
            choosing = false;
            await load();
          })}>Forget</Button
      >
    {/snippet}
  </SettingsCard>
{/if}

{#if overview && step === 'choose'}
  <SettingsCard
    title="People"
    description="Choose who receives each Jellyfin account’s state."
  >
    {#if !accounts}
      <div class={cardBlockClass}>
        {#if error}<Button
            variant="secondary"
            size="form"
            onclick={() => void choose()}>Retry</Button
          >{:else}<ContentSkeleton
            label="Loading Jellyfin accounts"
            count={2}
          />{/if}
      </div>
    {:else}
      <div class="overflow-x-auto">
        <table class={cardTableClass}>
          <thead>
            <tr>
              <th>Jellyfin account</th>
              <th class="text-right compact:hidden">Played</th>
              <th class="text-right whitespace-nowrap compact:hidden"
                >In progress</th
              >
              <th class="text-right compact:hidden">Favorites</th>
              <th class="w-60 compact:w-40">Import into</th>
            </tr>
          </thead>
          <tbody>
            {#each accounts as account (account.id)}
              <tr>
                <td>
                  <span class="font-[550]">{account.name}</span>
                  {#if account.administrator || account.disabled}<small
                      >{account.disabled
                        ? 'Disabled in Jellyfin'
                        : 'Administrator'}</small
                    >{/if}
                  <!-- Phones keep the person choice in view and list the counts here. -->
                  <small class="hidden compact:block"
                    >{n(account.played)} played · {n(account.in_progress)} in progress
                    · {n(account.favorites)} favorites</small
                  >
                </td>
                <td class="text-right tabular-nums compact:hidden"
                  >{n(account.played)}</td
                >
                <td class="text-right tabular-nums compact:hidden"
                  >{n(account.in_progress)}</td
                >
                <td class="text-right tabular-nums compact:hidden"
                  >{n(account.favorites)}</td
                >
                <td>
                  <select
                    class={formControlClass}
                    aria-label={`Import ${account.name} into`}
                    bind:value={mapping[account.id]}
                    disabled={previewing}
                  >
                    <option value="">Skip</option>
                    {#each people as person (person.id)}
                      <option value={person.id}>{person.username}</option>
                    {/each}
                  </select>
                  {#if !account.suggested && !mapping[account.id]}<small
                      class="mt-1.5"
                      >No one here is named {account.name}.
                      <button
                        type="button"
                        class="text-accent underline-offset-2 hover:underline"
                        onclick={() => openSettings('people')}
                        >Add them in People</button
                      ></small
                    >{/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </SettingsCard>
  <SettingsCard
    title="What to import"
    description="Nothing already watched or saved here is removed."
  >
    <SettingRow
      label="Watched"
      description="Movies and episodes played in Jellyfin."
      for={`${ids}-watched`}
    >
      <Switch id={`${ids}-watched`} bind:checked={wanted.watched} />
    </SettingRow>
    <SettingRow
      label="Resume points"
      description="Unfinished movies and episodes. The more recent position wins."
      for={`${ids}-resume`}
    >
      <Switch id={`${ids}-resume`} bind:checked={wanted.resume} />
    </SettingRow>
    <SettingRow
      label="Favorites"
      description="Movies, shows and episodes."
      for={`${ids}-favorites`}
    >
      <Switch id={`${ids}-favorites`} bind:checked={wanted.favorites} />
    </SettingRow>
    {#snippet footer()}
      {#if overview?.last}<Button
          variant="ghost"
          size="form"
          disabled={previewing}
          onclick={() => (choosing = false)}>Cancel</Button
        >{:else}<span></span>{/if}
      <Button
        loading={previewing}
        disabled={!selected.length ||
          !(wanted.watched || wanted.resume || wanted.favorites)}
        onclick={async () => {
          previewing = true;
          error = '';
          try {
            const value = await api<Report>(
              '/admin/jellyfin-import/preview',
              'POST',
              request(),
            );
            excluded.clear();
            showMissing = false;
            preview = value;
          } catch (caught) {
            error = String(caught);
          } finally {
            previewing = false;
          }
        }}>Preview</Button
      >
    {/snippet}
  </SettingsCard>
{/if}

{#if overview && (step === 'connect' || step === 'choose')}
  {@const library = overview.library}
  <SettingsCard
    title="Matching"
    description="Items match by file first, then by Radarr and Sonarr IDs, then by title."
  >
    <SettingRow
      label="Library"
      description={`${n(library.movies)} movies · ${n(library.episodes)} episodes${library.scanned_at ? ` · scanned ${time(library.scanned_at)}` : ''}`}
    >
      <StatusChip tone={library.movies || library.episodes ? 'ok' : 'warn'}
        >{library.movies || library.episodes ? 'Ready' : 'Empty'}</StatusChip
      >
    </SettingRow>
    {#each managers as manager (manager.kind)}
      <SettingRow
        label={manager.label}
        description={manager.source.connected
          ? `${manager.identified} · ${n(manager.source.identified)} of ${n(manager.source.titles)} ${manager.titles} identified`
          : manager.missing}
      >
        {#if manager.source.connected}<StatusChip tone="ok"
            >Connected</StatusChip
          >{:else}<StatusChip tone="warn">Not connected</StatusChip>
          <Button
            variant="secondary"
            size="sm"
            onclick={() => openSettings('services', manager.kind)}
            >Media services <ArrowRight size={13} aria-hidden="true" /></Button
          >{/if}
      </SettingRow>
    {/each}
  </SettingsCard>
{/if}

{#if step === 'preview' && preview}
  {@const groups = preview.groups ?? []}
  <SettingsCard
    title="Preview"
    description={`Read from ${preview.server} ${time(preview.at)}.`}
  >
    {#snippet badge()}<StatusChip tone="info">Not imported yet</StatusChip
      >{/snippet}
    {#snippet actions()}
      <Button
        variant="secondary"
        size="sm"
        disabled={busy}
        onclick={() => {
          preview = null;
        }}>Change</Button
      >
      <Button
        size="sm"
        loading={busy}
        onclick={() =>
          act(async () => {
            const imported = await withVerification(async () => {
              await api('/admin/jellyfin-import', 'POST', {
                ...request(),
                excluded: [...excluded],
              });
            });
            if (!imported) return;
            preview = null;
            choosing = false;
            showMissing = false;
            await load();
          })}>Import</Button
      >
    {/snippet}
    {@render counts(previewed, true)}
    {#if preview.retention}
      <div class={cardBlockClass}>
        <Notice tone="warning" class="mb-0">
          <p>
            <strong>Auto-delete is on.</strong>
            {excluded.size ? 'Up to ' : ''}{n(preview.retention)}
            {preview.retention === 1 ? 'title becomes' : 'titles become'} eligible
            and {preview.retention === 1 ? 'is' : 'are'} removed after the grace period.
          </p>
          <Button
            variant="secondary"
            size="sm"
            onclick={() => openSettings('retention')}
            >Auto-delete settings</Button
          >
        </Notice>
      </div>
    {/if}
  </SettingsCard>
  {#if groups.length}
    <SettingsCard
      title="Matched by title"
      description="No file or ID matched these. Uncheck any that are wrong."
    >
      {#snippet badge()}<StatusChip tone="warn">Check</StatusChip>{/snippet}
      <div class="overflow-x-auto">
        <table class={cardTableClass}>
          <thead>
            <tr>
              <th class="w-10"><span class="sr-only">Import</span></th>
              <th>In Jellyfin</th>
              <th>Here</th>
              <th class="text-right">Items</th>
            </tr>
          </thead>
          <tbody>
            {#each groups as group (group.key)}
              <tr>
                <td
                  ><input
                    type="checkbox"
                    class={formControlClass}
                    aria-label={`Import ${group.jellyfin}`}
                    checked={!excluded.has(group.key)}
                    disabled={busy}
                    onchange={(event) => {
                      if (event.currentTarget.checked)
                        excluded.delete(group.key);
                      else excluded.add(group.key);
                    }}
                  /></td
                >
                <td>{group.jellyfin}</td>
                <td>{group.title}</td>
                <td class="text-right whitespace-nowrap"
                  >{group.series
                    ? `${n(group.items)} ${group.items === 1 ? 'episode' : 'episodes'}`
                    : 'Movie'}</td
                >
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </SettingsCard>
  {/if}
  {#if preview.not_found.length}
    {@const rows = preview.not_found.length}
    <SettingsCard
      title="Not found"
      description="These keep their state here. Import again once they are in the library."
      footer={rows > 5 ? missingToggle : undefined}
    >
      {@render missingTable(preview.not_found, showMissing ? rows : 5)}
    </SettingsCard>
    {#snippet missingToggle()}
      <Button
        variant="ghost"
        size="sm"
        onclick={() => (showMissing = !showMissing)}
        >{#if showMissing}Show fewer <ChevronUp
            size={13}
            aria-hidden="true"
          />{:else}Show all {n(rows)}
          <ChevronDown size={13} aria-hidden="true" />{/if}</Button
      >
    {/snippet}
  {/if}
{/if}

{#if step === 'done' && overview?.last}
  {@const last = overview.last}
  <SettingsCard
    title="Last import"
    description={`${time(last.at)} from ${last.server}.`}
    footer={last.not_found.length ? reportToggle : undefined}
  >
    {#snippet badge()}<StatusChip tone="ok">Complete</StatusChip>{/snippet}
    {#snippet actions()}
      <Button variant="secondary" size="sm" onclick={() => void choose()}
        ><RefreshCw size={13} aria-hidden="true" /> Import again</Button
      >
    {/snippet}
    {@render counts(last.accounts, false)}
    {#if showMissing}{@render missingTable(
        last.not_found,
        last.not_found.length,
      )}{/if}
  </SettingsCard>
  {#snippet reportToggle()}
    <Button
      variant="ghost"
      size="sm"
      onclick={() => (showMissing = !showMissing)}
      >{#if showMissing}Hide not found <ChevronUp
          size={13}
          aria-hidden="true"
        />{:else}Show not found <ChevronDown
          size={13}
          aria-hidden="true"
        />{/if}</Button
    >
  {/snippet}
{/if}
