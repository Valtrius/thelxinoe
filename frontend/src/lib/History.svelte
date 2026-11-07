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
  import { dateFormat } from './intl';
  import { display } from './display.svelte';
  import { LatestRequest } from './latest-request';
  import { time } from './playback';
  import type { StatisticsPlatform, StatisticsRange } from './statistics/types';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import FormField from './ui/FormField.svelte';
  import { RowMotion } from './ui/row-motion.svelte';
  import RowMotionBody from './ui/RowMotionBody.svelte';
  import {
    badgeClass,
    cardBlockClass,
    formControlClass,
    quietDisabledClass,
  } from './ui/styles';
  // `active` is false while a page keeps this mounted out of sight. It reads
  // again on coming back, in case something changed in the meantime.
  let {
    user,
    audit = false,
    active = true,
    scope = 'mine',
    range = '30d',
    platform = 'all',
  } = $props<{
    user: User;
    audit?: boolean;
    active?: boolean;
    scope?: string;
    range?: StatisticsRange;
    platform?: StatisticsPlatform;
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
  const motion = new RowMotion();
  let auditUser = $state('');
  let auditAction = $state('');
  let auditGroup = $state<AuditCategory | ''>('');
  let actors = $state<NonNullable<Result['actors']>>([]);
  let actions = $state<string[]>([]);
  let result = $state<Result | null>(null),
    error = $state(''),
    busy = $state(false);
  // The scope `result` was read for, which `scope` is ahead of during a read.
  let appliedScope = $state('mine');
  // The list `result` answers. A read of any other replaces the rows.
  let shownList: string | undefined;
  const groups = $derived(auditActionGroups(actions));
  const days = $derived(
    audit ? auditDays(result?.items ?? [], display.timezone) : [],
  );
  const hasRows = $derived((result?.items.length ?? 0) > 0);
  const requests = new LatestRequest();
  let reading: AbortController | undefined;
  onDestroy(() => {
    requests.invalidate();
    reading?.abort();
  });
  $effect(() => {
    void [audit, scope, range, platform, auditUser, auditAction, auditGroup];
    if (active) untrack(() => void load());
  });
  function selectGroup(group: AuditCategory | '') {
    auditGroup = group;
    if (group && auditAction && auditCategory(auditAction) !== group)
      auditAction = '';
  }
  async function load({ more = false } = {}) {
    reading?.abort();
    const { signal } = (reading = new AbortController());
    const current = requests.begin();
    const selectedScope: string = scope;
    busy = true;
    if (!more) motion.hold();
    const query = new SvelteURLSearchParams();
    if (!audit) {
      query.set('range', range);
      query.set('platform', platform);
    }
    if (audit) {
      if (auditUser) query.set('user', auditUser);
      // A category filters by every action key it currently contains.
      const filter =
        auditAction ||
        (auditGroup
          ? auditActionGroups(actions, auditGroup)
              .flatMap((group) => group.options.map((option) => option.value))
              .join(',')
          : '');
      if (filter) query.set('action', filter);
    }
    const administrative =
      !audit && user.role === 'admin' && selectedScope !== 'mine';
    if (administrative && selectedScope !== 'all')
      query.set('user', selectedScope);
    const path = audit
      ? '/admin/audit'
      : administrative
        ? '/admin/history'
        : '/me/history';
    const list = `${path}?${query}`;
    const replacing = list !== shownList;
    if (more && result?.next_before)
      query.set('before', String(result.next_before));
    try {
      const response = await api<Result>(
        `${path}?${query}`,
        'GET',
        undefined,
        signal,
      );
      if (current())
        await motion.commit(
          () => {
            actors = response.actors ?? actors;
            actions = response.actions ?? actions;
            appliedScope = selectedScope;
            shownList = list;
            error = '';
            result = more
              ? {
                  ...response,
                  items: [...(result?.items ?? []), ...response.items],
                }
              : response;
          },
          { live: !replacing && !more },
        );
    } catch (e) {
      if (current())
        await motion.commit(() => {
          // Rows that do not match the filters cannot stay on show.
          if (replacing) {
            result = null;
            shownList = undefined;
          }
          error = String(e);
        });
    } finally {
      if (current()) busy = false;
    }
  }
  function date(value?: number) {
    return value === undefined
      ? ''
      : dateFormat('en', {
          dateStyle: 'medium',
          timeStyle: 'short',
          timeZone: display.timezone,
          hour12: display.timeFormat === '12h',
        }).format(value * 1000);
  }
  const clockFormat = $derived(
    new Intl.DateTimeFormat('en', {
      hour: display.timeFormat === '12h' ? 'numeric' : '2-digit',
      minute: '2-digit',
      hourCycle: display.timeFormat === '12h' ? 'h12' : 'h23',
      timeZone: display.timezone,
    }),
  );
  const stampFormat = $derived(
    new Intl.DateTimeFormat('en', {
      dateStyle: 'full',
      timeStyle: 'long',
      timeZone: display.timezone,
      hour12: display.timeFormat === '12h',
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

{#snippet auditTable()}
  <!-- Pinned columns stay put between filters. When they stop fitting, each entry stacks into two lines. -->
  <table
    class="w-full table-fixed border-separate border-spacing-0 text-left text-xs table-stack:block"
  >
    <caption class="sr-only">Administrative activity</caption>
    <colgroup class="table-stack:hidden">
      <col class="w-23" />
      <col class="w-38" />
      <col class="w-84" />
      <col />
    </colgroup>
    <thead
      class="border-b border-line bg-surface-soft text-[0.62rem] text-muted table-stack:hidden"
      data-motion-row="head"
    >
      <tr>
        <th scope="col" class="px-5.5 py-2 font-medium">Time</th>
        <th scope="col" class="px-3 py-2 font-medium">User</th>
        <th scope="col" class="px-3 py-2 font-medium">Event</th>
        <th scope="col" class="px-3 py-2 font-medium">Target</th>
      </tr>
    </thead>
    {#each days as day, index (day.key)}
      {@const partial = index === days.length - 1 && !!result?.next_before}
      <tbody class="table-stack:block">
        <tr class="table-stack:block" data-motion-row={`day:${day.key}`}>
          <th
            scope="colgroup"
            colspan="4"
            class="bg-surface-strong px-5.5 py-1.5 text-[0.7rem] font-semibold tracking-normal text-foreground normal-case compact:px-4 table-stack:block"
          >
            {day.label}<span class="ml-2 font-normal text-muted"
              >{day.rows.length}{partial ? '+' : ''}
              {day.rows.length === 1 && !partial ? 'event' : 'events'}</span
            >
          </th>
        </tr>
        {#each day.rows as row (`audit:${row.id}`)}
          <tr
            data-motion-row={`audit:${row.id}`}
            class="border-b border-line compact:px-4 table-stack:grid table-stack:grid-cols-[auto_minmax(0,1fr)_auto] table-stack:items-center table-stack:gap-x-2 table-stack:gap-y-1 table-stack:px-5.5 table-stack:py-2"
          >
            <td
              class="px-5.5 py-1.5 whitespace-nowrap tabular-nums table-stack:order-2 table-stack:border-0 table-stack:p-0 table-stack:text-muted"
              title={stampFormat.format((row.created_at ?? 0) * 1000)}
            >
              {clockFormat.format((row.created_at ?? 0) * 1000)}
            </td>
            <td
              class="px-3 py-1.5 table-stack:order-3 table-stack:border-0 table-stack:p-0"
            >
              <span class="flex min-w-0 items-center gap-2">
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
                <span
                  class={['truncate', !row.actor && 'text-muted italic']}
                  title={row.actor ?? undefined}
                  >{row.actor ??
                    (row.actor_id ? 'Deleted user' : 'System')}</span
                >
              </span>
            </td>
            <td
              class="px-3 py-1.5 table-stack:order-1 table-stack:col-span-2 table-stack:border-0 table-stack:p-0 table-stack:font-medium"
              title={row.action}
            >
              {auditLabel(row.action ?? '')}<span
                class="{badgeClass} ml-2 align-middle font-normal"
                >{auditCategory(row.action ?? '')}</span
              >
            </td>
            <td
              class="px-3 py-1.5 font-mono text-[0.6875rem] text-muted wrap-anywhere table-stack:order-4 table-stack:col-span-2 table-stack:border-0 table-stack:p-0"
            >
              {row.target}
            </td>
          </tr>
        {/each}
      </tbody>
    {/each}
  </table>
{/snippet}

{#snippet auditFilters()}
  <div class="{cardBlockClass} flex flex-wrap items-end gap-4">
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
    <FormField class="mb-0 w-48"
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
    <FormField class="mb-0 w-56"
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
{/snippet}

{#snippet state()}
  {#if !result}
    {#if motion.stale}
      <div class={audit ? cardBlockClass : ''}>
        <ContentSkeleton
          label={audit
            ? 'Loading administrative activity'
            : 'Loading playback history'}
        />
      </div>
    {/if}
  {:else if !hasRows && !error}
    <p
      class={['text-muted', audit && `m-0 ${cardBlockClass} text-xs`]}
      role="status"
      data-motion-row="empty"
    >
      {audit && (auditUser || auditAction || auditGroup)
        ? 'No activity matches these filters.'
        : 'No activity in this view yet.'}
    </p>
  {/if}
{/snippet}

{#if error}<Notice role="alert" variant="error" class={audit ? 'mb-0' : ''}
    >{error}</Notice
  >{/if}
{#if audit}
  <SettingsCard
    title="Administrative activity"
    description={`Who changed what, grouped by day. Times shown in ${display.timezone}.`}
  >
    {#snippet actions()}
      <Button
        variant="secondary"
        size="form"
        disabled={motion.stale}
        onclick={() => void load()}>Refresh</Button
      >
    {/snippet}
    {@render auditFilters()}
    <RowMotionBody {motion} loaded={!!result} class="divide-y divide-line">
      {#if hasRows}
        {@render auditTable()}
      {/if}
      {@render state()}
      {#if result?.next_before}
        <div class={cardBlockClass} data-motion-row="older">
          <Button
            variant="secondary"
            size="form"
            class={quietDisabledClass}
            disabled={busy}
            onclick={() => void load({ more: true })}
            >Load older activity</Button
          >
        </div>
      {/if}
    </RowMotionBody>
  </SettingsCard>
{:else}
  <Panel>
    <SectionHeading>
      <h2>Playback history</h2>
      <Button
        variant="secondary"
        size="form"
        disabled={motion.stale}
        onclick={() => void load()}>Refresh</Button
      >
    </SectionHeading>
    <RowMotionBody {motion} loaded={!!result}>
      {#if hasRows}
        <!-- Clips vertically so that rows moving past its edge cannot add a scrollbar. -->
        <div class="min-w-0 overflow-x-auto overflow-y-hidden">
          <table
            class="w-full min-w-[60rem] table-fixed border-separate border-spacing-0 text-left text-xs"
          >
            <caption class="sr-only">Playback history</caption>
            <colgroup>
              <col />
              {#if appliedScope !== 'mine'}<col class="w-28" />{/if}
              <col class="w-40" />
              <col class="w-20" />
              <col class="w-32" />
              <col class="w-32" />
              <col class="w-32" />
              <col class="w-18" />
              <col class="w-20" />
            </colgroup>
            <thead
              class="border-b border-line bg-surface-soft text-[0.62rem] text-muted"
              data-motion-row="head"
            >
              <tr>
                <th scope="col" class="px-3 py-2 font-medium">Title</th>
                {#if appliedScope !== 'mine'}
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
                <tr
                  class="border-b border-line last:border-b-0"
                  data-motion-row={`${row.kind ?? 'media'}:${row.id}`}
                >
                  <td class="px-3 py-3 font-medium wrap-anywhere">
                    {row.title}
                  </td>
                  {#if appliedScope !== 'mine'}
                    <td class="truncate px-3 py-3" title={row.username}>
                      {row.username}
                    </td>
                  {/if}
                  <td class="px-3 py-3 whitespace-nowrap">
                    {date(row.started_at)}
                  </td>
                  <td class="px-3 py-3 whitespace-nowrap">{kind(row.kind)}</td>
                  <td class="truncate px-3 py-3" title={row.device}>
                    {row.device}
                  </td>
                  <td
                    class="truncate px-3 py-3"
                    title={row.edition || 'Original edition'}
                  >
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
                  <td class="truncate px-3 py-3" title={row.state}>
                    {row.state}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
      {@render state()}
      {#if result?.next_before}
        <div data-motion-row="older">
          <Button
            variant="secondary"
            size="form"
            class={quietDisabledClass}
            disabled={busy}
            onclick={() => void load({ more: true })}
            >Load older activity</Button
          >
        </div>
      {/if}
    </RowMotionBody>
  </Panel>
{/if}
