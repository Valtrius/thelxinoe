<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import { api, type User } from './api';
  import { LatestRequest } from './providers/latest-request';
  import { time } from './playback';
  import type { StatisticsPlatform, StatisticsRange } from './statistics/types';
  import Button from './ui/Button.svelte';
  import Panel from './ui/Panel.svelte';
  import { errorClass, rowClass, sectionHeadingClass } from './ui/styles';
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

{#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
<Panel>
  <div class={sectionHeadingClass}>
    <h2>
      {audit ? 'Administrative activity' : 'Playback history'}
    </h2>
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load()}>Refresh</Button
    >
  </div>
  <p class="text-muted">Times shown in {user.timezone}.</p>
  {#each result?.items ?? [] as row (`${row.kind ?? 'audit'}:${row.id}`)}<div
      class={rowClass}
    >
      <div>
        <strong>{audit ? row.action : row.title}</strong><small
          >{date(audit ? row.created_at : row.started_at)}{audit
            ? ` · ${row.actor ?? 'Deleted user'}`
            : scope !== 'mine'
              ? ` · ${row.username}`
              : ''}</small
        ><small
          >{audit
            ? row.target
            : `${kind(row.kind)} · ${row.device} · ${row.edition || 'Original edition'}`}</small
        >
      </div>
      {#if !audit}<span>
          {#if (row.duration ?? 0) > 0}
            {time(row.position ?? 0)} / {time(row.duration ?? 0)}
          {:else}
            {time(row.played_seconds ?? 0)}
          {/if}
        </span><small
          >{time(row.played_seconds ?? 0)} played · {row.state}</small
        >{/if}
    </div>{:else}<p class="text-muted">No activity in this view yet.</p>{/each}
  {#if result?.next_before}<Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load(audit, true)}>Load older activity</Button
    >{/if}
</Panel>
