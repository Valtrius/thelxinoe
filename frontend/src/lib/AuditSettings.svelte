<script lang="ts">
  import { untrack } from 'svelte';
  import { api, type User } from './api';
  import History from './History.svelte';
  import { formatTimestamp } from './display.svelte';
  import { LatestRequest } from './latest-request';
  import { RefreshTask } from './refresh-task';
  import Button from './ui/Button.svelte';
  import { RowMotion } from './ui/row-motion.svelte';
  import RowMotionBody from './ui/RowMotionBody.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import {
    cardBlockClass,
    formControlClass,
    quietDisabledClass,
  } from './ui/styles';
  import { twMerge } from 'tailwind-merge';

  let { user }: { user: User } = $props();
  type Item = {
    id: string;
    title: string;
    state: string;
    reason: string;
    due_at: number;
    eligible_at?: number;
    error: string | null;
    watched_users?: string[];
  };
  type Totals = { pending: number; history: number };
  const ids = $props.id();
  const viewMotion = new RowMotion();
  const listMotion = new RowMotion();
  // What the select shows moves at once; what the page shows waits for its data.
  let selected = $state('activity');
  let view = $state('activity');
  let items = $state<Item[]>([]);
  let totals = $state<Totals>({ pending: 0, history: 0 });
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state('');
  const requests = new LatestRequest();
  let reads: RefreshTask | undefined;
  const scheduled = $derived(view === 'scheduled');
  const visible = $derived(
    items
      .filter((item) =>
        scheduled ? item.state === 'pending' : item.state !== 'pending',
      )
      .sort((a, b) =>
        scheduled
          ? a.due_at - b.due_at
          : (b.eligible_at ?? b.due_at) - (a.eligible_at ?? a.due_at),
      ),
  );
  const total = $derived(scheduled ? totals.pending : totals.history);
  const n = (value: number) => value.toLocaleString();
  const reason = (item: Item) =>
    item.reason === 'storage_limit'
      ? 'Storage limit'
      : `Watched${item.watched_users?.length ? ' by ' + item.watched_users.join(', ') : ''}`;
  const due = (item: Item) =>
    `${item.state === 'pending' && item.due_at * 1000 <= Date.now() ? 'Overdue since' : 'Scheduled for'} ${formatTimestamp(item.due_at)}`;
  const stateLabel = (state: string) =>
    ({
      complete: 'Deleted',
      kept: 'Kept',
      cancelled: 'Unscheduled',
      blocked: 'Needs review',
      executing: 'Deleting',
    })[state] ?? state;
  const stateTone = (state: string) =>
    state === 'complete'
      ? 'ok'
      : state === 'blocked'
        ? 'bad'
        : state === 'executing'
          ? 'info'
          : 'neutral';

  async function refresh(signal: AbortSignal) {
    const current = requests.begin();
    const next = selected;
    try {
      const result = await api<{ items: Item[]; totals: Totals }>(
        '/admin/retention/deletions',
        'GET',
        undefined,
        signal,
      );
      if (!current()) return;
      const apply = () => {
        items = result.items;
        totals = result.totals;
        loaded = true;
        error = '';
        view = next;
      };
      if (view === next) {
        viewMotion.release();
        await listMotion.commit(apply, { live: true });
      } else {
        listMotion.settle();
        await viewMotion.commit(apply, { dissolve: true });
      }
    } catch (failure) {
      if (!current()) return;
      viewMotion.release();
      error = String(failure);
      view = next;
    }
  }
  async function act(item: Item, action: 'keep' | 'delete') {
    if (
      action === 'delete' &&
      !confirm(
        `Delete ${item.title} now? This permanently removes its downloaded media files.`,
      )
    )
      return;
    busy = true;
    error = '';
    requests.invalidate();
    try {
      await api(`/admin/retention/${item.id}/${action}`, 'POST');
      await reads?.refresh();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
  function select(next: string) {
    selected = next;
    if (next !== 'activity') {
      viewMotion.hold();
      return;
    }
    requests.invalidate();
    if (view === next) viewMotion.release();
    else void viewMotion.commit(() => (view = next), { dissolve: true });
  }
  $effect(() => {
    if (selected === 'activity') return;
    const task = new RefreshTask(refresh, 30_000, () => !busy);
    reads = task;
    untrack(() => task.start());
    return () => {
      task.stop();
      requests.invalidate();
    };
  });
</script>

<div class="col-span-full flex flex-wrap items-center gap-3">
  <label for={`${ids}-view`} class="text-xs font-[550]">View</label>
  <select
    id={`${ids}-view`}
    aria-label="Audit view"
    class={twMerge(
      formControlClass,
      quietDisabledClass,
      'h-8.5 w-64 py-1.5 text-xs',
    )}
    value={selected}
    onchange={(event) => select(event.currentTarget.value)}
    disabled={busy}
  >
    <option value="activity">Administrative activity</option>
    <option value="scheduled">Scheduled deletions</option>
    <option value="deletions">Deletion history</option>
  </select>
</div>
<RowMotionBody motion={viewMotion} class="grid gap-5">
  <!-- Stays mounted, so that coming back shows its rows and filters at once. -->
  <div class="grid min-w-0 gap-5" hidden={view !== 'activity'}>
    <History {user} audit active={view === 'activity'} />
  </div>
  {#if view !== 'activity'}
    {#if error}<p class="m-0 text-xs text-danger" role="alert">
        {error}
      </p>{/if}
    <SettingsCard
      aria-label={scheduled ? 'Scheduled deletions' : 'Deletion history'}
      title={scheduled ? 'Scheduled deletions' : 'Deletion history'}
      description={scheduled
        ? 'Media waiting for its grace period to end. Keep cancels a deletion; Delete now skips the wait.'
        : 'Outcomes of past automatic deletions.'}
    >
      {#snippet actions()}
        <span class="text-xs text-muted"
          >{total > visible.length ? `${n(visible.length)} of ` : ''}{n(total)}
          {total === 1 ? 'item' : 'items'}</span
        >
      {/snippet}
      <RowMotionBody motion={listMotion} class="divide-y divide-line">
        {#if !loaded}
          <p
            class="m-0 {cardBlockClass} text-xs text-muted"
            role="status"
            data-motion-row="unavailable"
          >
            Deletions could not be loaded.
          </p>
        {:else}
          {#each visible as item (item.id)}
            <SettingRow
              data-motion-row={item.id}
              label={item.title}
              description={`${reason(item)} · ${due(item)}${item.error ? ` · ${item.error}` : ''}`}
            >
              {#if scheduled}
                <Button
                  variant="secondary"
                  size="form"
                  class={quietDisabledClass}
                  disabled={busy}
                  onclick={() => act(item, 'keep')}>Keep</Button
                >
                <Button
                  variant="danger"
                  size="form"
                  class={quietDisabledClass}
                  disabled={busy}
                  onclick={() => act(item, 'delete')}>Delete now</Button
                >
              {:else}
                <StatusChip tone={stateTone(item.state)}
                  >{stateLabel(item.state)}</StatusChip
                >
              {/if}
            </SettingRow>
          {:else}
            <p
              class="m-0 {cardBlockClass} text-xs text-muted"
              data-motion-row="empty"
            >
              {scheduled ? 'No scheduled deletions.' : 'No deletion history.'}
            </p>
          {/each}
        {/if}
      </RowMotionBody>
    </SettingsCard>
  {/if}
</RowMotionBody>
