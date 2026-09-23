<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import SectionHeading from './ui/SectionHeading.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, type User } from './api';
  import { LatestRequest } from './providers/latest-request';
  import { time } from './playback';
  import type { StatisticsPlatform, StatisticsRange } from './statistics/types';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { rowClass } from './ui/styles';
  let {
    user,
    audit = false,
    scope = 'mine',
    range = '30d',
    platform = 'all',
  } = $props<{
    user: User;
    audit?: boolean;
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
    actor?: string;
  };
  type Result = {
    items: Row[];
    next_before: number | string | null;
  };
  let result = $state<Result | null>(null),
    error = $state(''),
    busy = $state(false);
  const requests = new LatestRequest();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    const mode = audit;
    const selectedScope = scope;
    const selectedRange = range;
    const selectedPlatform = platform;
    untrack(() => {
      void load(mode, false, selectedScope, selectedRange, selectedPlatform);
    });
  });
  async function load(
    mode = audit,
    more = false,
    selectedScope = scope,
    selectedRange = range,
    selectedPlatform = platform,
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
      if (more && result?.next_before)
        query.set('before', String(result.next_before));
      const administrative =
        !mode && user.role === 'admin' && selectedScope !== 'mine';
      if (administrative && selectedScope !== 'all')
        query.set('user', selectedScope);
      const response = await api<Result>(
        `${mode ? '/admin/audit' : administrative ? '/admin/history' : '/me/history'}?${query}`,
      );
      if (current())
        result = more
          ? {
              ...response,
              items: [...(result?.items ?? []), ...response.items],
            }
          : response;
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
        }).format(value * 1000);
  }
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
<Panel>
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
  {#if (result?.items.length ?? 0) > 0}
    {#if audit}
      {#each result?.items ?? [] as row (`audit:${row.id}`)}<div
          class={rowClass}
        >
          <div>
            <strong>{row.action}</strong><small
              >{date(row.created_at)} · {row.actor ?? 'Deleted user'}</small
            ><small>{row.target}</small>
          </div>
        </div>{/each}
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
  {:else}
    <p class="text-muted">No activity in this view yet.</p>
  {/if}
  {#if result?.next_before}<Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load(audit, true)}>Load older activity</Button
    >{/if}
</Panel>
