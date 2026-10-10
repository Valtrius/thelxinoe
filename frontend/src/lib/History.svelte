<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import SectionHeading from './ui/SectionHeading.svelte';
  import { Cog, UserX } from '@lucide/svelte';
  import { onDestroy, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, type User } from './api';
  import {
    auditActionGroups,
    auditCategory,
    auditDays,
    auditLabel,
    type AuditCategory,
  } from './audit-events';
  import { LatestRequest } from './latest-request';
  import { time } from './playback';
  import type { StatisticsPlatform, StatisticsRange } from './statistics/types';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import FormField from './ui/FormField.svelte';
  import { badgeClass, formControlClass } from './ui/styles';
  let {
    user,
    audit = false,
    scope = 'mine',
    range = '30d',
    platform = 'all',
    timeFormat = '24h',
  } = $props<{
    user: User;
    audit?: boolean;
    scope?: string;
    range?: StatisticsRange;
    platform?: StatisticsPlatform;
    timeFormat?: '12h' | '24h';
  }>();
  type Row = {
    id: number | string;
    kind?: string;
    title?: string;
    username?: string;
    device?: string;
    started_at?: number;
    created_at?: number;
    played_seconds?: number;
    position?: number;
    duration?: number;
    edition?: string;
    state?: string;
    action?: string;
    target?: string;
    actor?: string | null;
    actor_id?: string | null;
  };
  type Result = {
    items: Row[];
    next_before: number | string | null;
    actors?: { id: string; username: string }[];
    actions?: string[];
  };
  let auditUser = $state('');
  let auditAction = $state('');
  let auditGroup = $state<AuditCategory | ''>('');
  let actors = $state<NonNullable<Result['actors']>>([]);
  let actions = $state<string[]>([]);
  let result = $state<Result | null>(null),
    error = $state(''),
    busy = $state(false);
  const groups = $derived(auditActionGroups(actions));
  const days = $derived(
    audit ? auditDays(result?.items ?? [], user.timezone) : [],
  );
  const requests = new LatestRequest();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    const mode = audit;
    const selectedScope = scope;
    const selectedRange = range;
    const selectedPlatform = platform;
    const selectedUser = auditUser;
    const selectedAction = auditAction;
    const selectedGroup = auditGroup;
    untrack(() => {
      void load(
        mode,
        false,
        selectedScope,
        selectedRange,
        selectedPlatform,
        selectedUser,
        selectedAction,
        selectedGroup,
      );
    });
  });
  function selectGroup(group: AuditCategory | '') {
    auditGroup = group;
    if (group && auditAction && auditCategory(auditAction) !== group)
      auditAction = '';
  }
  async function load(
    mode = audit,
    more = false,
    selectedScope = scope,
    selectedRange = range,
    selectedPlatform = platform,
    selectedUser = auditUser,
    selectedAction = auditAction,
    selectedGroup = auditGroup,
  ) {
    const current = requests.begin();
    busy = true;
    error = '';
    if (!more) result = null;
    try {
      const query = new SvelteURLSearchParams();
      if (!mode) {
        query.set('range', selectedRange);
        query.set('platform', selectedPlatform);
      }
      if (mode) {
        if (selectedUser) query.set('user', selectedUser);
        // A category filters by every action key it currently contains.
        const filter =
          selectedAction ||
          (selectedGroup
            ? auditActionGroups(actions, selectedGroup)
                .flatMap((group) => group.options.map((option) => option.value))
                .join(',')
            : '');
        if (filter) query.set('action', filter);
      }
      if (more && result?.next_before)
        query.set('before', String(result.next_before));
      const administrative =
        !mode && user.role === 'admin' && selectedScope !== 'mine';
      if (administrative && selectedScope !== 'all')
        query.set('user', selectedScope);
      const response = await api<Result>(
        `${mode ? '/admin/audit' : administrative ? '/admin/history' : '/me/history'}?${query}`,
      );
      if (current()) {
        actors = response.actors ?? actors;
        actions = response.actions ?? actions;
        result = more
          ? {
              ...response,
              items: [...(result?.items ?? []), ...response.items],
            }
          : response;
      }
    } catch (e) {
      if (current()) error = String(e);
    } finally {
      if (current()) busy = false;
    }
  }
  function date(value?: number) {
    return value === undefined
      ? ''
      : new Intl.DateTimeFormat('en', {
          dateStyle: 'medium',
          timeStyle: 'short',
          timeZone: user.timezone,
          hour12: timeFormat === '12h',
        }).format(value * 1000);
  }
  const clockFormat = $derived(
    new Intl.DateTimeFormat('en', {
      hour: timeFormat === '12h' ? 'numeric' : '2-digit',
      minute: '2-digit',
      hourCycle: timeFormat === '12h' ? 'h12' : 'h23',
      timeZone: user.timezone,
    }),
  );
  const stampFormat = $derived(
    new Intl.DateTimeFormat('en', {
      dateStyle: 'full',
      timeStyle: 'long',
      timeZone: user.timezone,
      hour12: timeFormat === '12h',
    }),
  );
  function kind(value?: string) {
    switch (value) {
      case 'movie':
        return 'Movie';
      case 'episode':
        return 'Episode';
      case 'track':
        return 'Music';
      case 'youtube':
        return 'YouTube';
      case 'twitch':
        return 'Twitch';
      case 'kick':
        return 'Kick';
      default:
        return 'Media';
    }
  }
</script>

{#if error}<Notice role="alert" variant="error">{error}</Notice>{/if}
<Panel class={audit ? 'settings-wide' : undefined}>
  <SectionHeading>
    <h2>
      {audit ? 'Administrative activity' : 'Playback history'}
    </h2>
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load()}>Refresh</Button
    >
  </SectionHeading>
  {#if audit}<p class="text-muted">Times shown in {user.timezone}.</p>{/if}
  {#if audit}
    <div class="my-5 flex flex-wrap items-end gap-4">
      {#if groups.length > 1}
        <div
          class="flex max-w-full min-w-0 flex-col gap-2 text-[11px] font-[550]"
        >
          <span aria-hidden="true">Category</span>
          <div class="max-w-full overflow-x-auto">
            <ExclusiveChoiceGroup
              ariaLabel="Audit category"
              value={auditGroup}
              choices={[
                { value: '', label: 'All' },
                ...groups.map((group) => ({
                  value: group.category,
                  label: group.category,
                })),
              ]}
              onChange={selectGroup}
            />
          </div>
        </div>
      {/if}
      <FormField class="mb-0"
        >User<select
          aria-label="Audit user"
          class={formControlClass}
          bind:value={auditUser}
        >
          <option value="">All users</option>
          {#each actors as actor (actor.id)}<option value={actor.id}
              >{actor.username}</option
            >{/each}
        </select></FormField
      >
      <FormField class="mb-0"
        >Action<select
          aria-label="Audit action"
          class={formControlClass}
          bind:value={auditAction}
        >
          <option value="">All actions</option>
          {#each auditActionGroups(actions, auditGroup) as group (group.category)}
            <optgroup label={group.category}>
              {#each group.options as option (option.value)}<option
                  value={option.value}>{option.label}</option
                >{/each}
            </optgroup>
          {/each}
        </select></FormField
      >
    </div>
  {/if}
  {#if (result?.items.length ?? 0) > 0}
    {#if audit}
      <!-- Below the compact breakpoint each entry stacks into two lines. -->
      <table class="w-full border-collapse text-left text-xs compact:block">
        <caption class="sr-only">Administrative activity</caption>
        <thead
          class="border-b border-line bg-surface-soft text-[0.62rem] text-muted compact:hidden"
        >
          <tr>
            <th scope="col" class="w-0 px-3 py-2 font-medium">Time</th>
            <th scope="col" class="w-0 px-3 py-2 font-medium">User</th>
            <th scope="col" class="w-0 px-3 py-2 font-medium">Event</th>
            <th scope="col" class="px-3 py-2 font-medium">Target</th>
          </tr>
        </thead>
        {#each days as day, index (day.key)}
          {@const partial = index === days.length - 1 && !!result?.next_before}
          <tbody class="compact:block">
            <tr class="compact:block">
              <th
                scope="colgroup"
                colspan="4"
                class="bg-surface-strong px-3 py-1.5 text-[0.7rem] font-semibold tracking-normal text-foreground normal-case compact:block"
              >
                {day.label}<span class="ml-2 font-normal text-muted"
                  >{day.rows.length}{partial ? '+' : ''}
                  {day.rows.length === 1 && !partial ? 'event' : 'events'}</span
                >
              </th>
            </tr>
            {#each day.rows as row (`audit:${row.id}`)}
              <tr
                class="border-b border-line compact:grid compact:grid-cols-[auto_minmax(0,1fr)_auto] compact:items-center compact:gap-x-2 compact:gap-y-1 compact:py-2"
              >
                <td
                  class="px-3 py-1.5 whitespace-nowrap tabular-nums compact:order-2 compact:border-0 compact:p-0 compact:text-muted"
                  title={stampFormat.format((row.created_at ?? 0) * 1000)}
                >
                  {clockFormat.format((row.created_at ?? 0) * 1000)}
                </td>
                <td
                  class="px-3 py-1.5 whitespace-nowrap compact:order-3 compact:border-0 compact:p-0"
                >
                  <span class="inline-flex items-center gap-2 align-middle">
                    <span
                      aria-hidden="true"
                      class={[
                        'grid size-5 shrink-0 place-items-center border text-[0.625rem] font-semibold',
                        row.actor
                          ? 'border-line-strong bg-accent-soft text-accent'
                          : 'border-dashed border-line text-muted',
                      ]}
                    >
                      {#if row.actor}{row.actor.charAt(0).toUpperCase()}
                      {:else if row.actor_id}<UserX size={12} />
                      {:else}<Cog size={12} />{/if}
                    </span>
                    <!-- No actor_id is a system event; an unmatched one was deleted. -->
                    <span class={row.actor ? undefined : 'text-muted italic'}
                      >{row.actor ??
                        (row.actor_id ? 'Deleted user' : 'System')}</span
                    >
                  </span>
                </td>
                <td
                  class="px-3 py-1.5 whitespace-nowrap compact:order-1 compact:col-span-2 compact:border-0 compact:p-0 compact:font-medium compact:whitespace-normal"
                  title={row.action}
                >
                  {auditLabel(row.action ?? '')}<span
                    class="{badgeClass} ml-2 align-middle font-normal"
                    >{auditCategory(row.action ?? '')}</span
                  >
                </td>
                <td
                  class="px-3 py-1.5 font-mono text-[0.6875rem] text-muted wrap-anywhere compact:order-4 compact:col-span-2 compact:border-0 compact:p-0"
                >
                  {row.target}
                </td>
              </tr>
            {/each}
          </tbody>
        {/each}
      </table>
    {:else}
      <div class="min-w-0 overflow-x-auto">
        <table class="w-full min-w-[56rem] border-collapse text-left text-xs">
          <caption class="sr-only">Playback history</caption>
          <thead
            class="border-b border-line bg-surface-soft text-[0.62rem] text-muted"
          >
            <tr>
              <th scope="col" class="px-3 py-2 font-medium">Title</th>
              {#if scope !== 'mine'}
                <th scope="col" class="px-3 py-2 font-medium">User</th>
              {/if}
              <th scope="col" class="px-3 py-2 font-medium">Started</th>
              <th scope="col" class="px-3 py-2 font-medium">Type</th>
              <th scope="col" class="px-3 py-2 font-medium">Device</th>
              <th scope="col" class="px-3 py-2 font-medium">Edition</th>
              <th scope="col" class="px-3 py-2 font-medium">Progress</th>
              <th scope="col" class="px-3 py-2 font-medium">Played</th>
              <th scope="col" class="px-3 py-2 font-medium">State</th>
            </tr>
          </thead>
          <tbody>
            {#each result?.items ?? [] as row (`${row.kind ?? 'media'}:${row.id}`)}
              <tr class="border-b border-line last:border-b-0">
                <td class="max-w-72 px-3 py-3 font-medium wrap-anywhere">
                  {row.title}
                </td>
                {#if scope !== 'mine'}
                  <td class="px-3 py-3 whitespace-nowrap">{row.username}</td>
                {/if}
                <td class="px-3 py-3 whitespace-nowrap">
                  {date(row.started_at)}
                </td>
                <td class="px-3 py-3 whitespace-nowrap">{kind(row.kind)}</td>
                <td class="px-3 py-3 whitespace-nowrap">{row.device}</td>
                <td class="px-3 py-3 whitespace-nowrap">
                  {row.edition || 'Original edition'}
                </td>
                <td class="px-3 py-3 whitespace-nowrap tabular-nums">
                  {#if (row.duration ?? 0) > 0}
                    {time(row.position ?? 0)} / {time(row.duration ?? 0)}
                  {:else}
                    {time(row.played_seconds ?? 0)}
                  {/if}
                </td>
                <td class="px-3 py-3 whitespace-nowrap tabular-nums">
                  {time(row.played_seconds ?? 0)}
                </td>
                <td class="px-3 py-3 whitespace-nowrap">{row.state}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {:else if busy}
    <ContentSkeleton
      label={audit
        ? 'Loading administrative activity'
        : 'Loading playback history'}
    />
  {:else if !error}
    <p class="text-muted" role="status">
      {audit && (auditUser || auditAction || auditGroup)
        ? 'No activity matches these filters.'
        : 'No activity in this view yet.'}
    </p>
  {/if}
  {#if result?.next_before}<Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load(audit, true)}>Load older activity</Button
    >{/if}
</Panel>
