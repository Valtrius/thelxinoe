<script lang="ts">
  import { untrack } from 'svelte';
  import { api, type User } from './api';
  import History from './History.svelte';
  import { LatestRequest } from './providers/latest-request';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Panel from './ui/Panel.svelte';
  import { formControlClass } from './ui/styles';

  let { user, timeFormat }: { user: User; timeFormat: '12h' | '24h' } =
    $props();
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
  let view = $state('activity');
  let items = $state<Item[]>([]);
  let loaded = $state(false);
  let busy = $state(false);
  let error = $state('');
  const requests = new LatestRequest();
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
  const date = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(undefined, {
      timeZone: user.timezone,
      hour12: timeFormat === '12h',
    });
  const reason = (item: Item) =>
    item.reason === 'storage_limit'
      ? 'Storage limit'
      : `Watched${item.watched_users?.length ? ' by ' + item.watched_users.join(', ') : ''}`;
  const stateLabel = (state: string) =>
    ({
      complete: 'Deleted',
      kept: 'Kept',
      cancelled: 'Unscheduled',
      blocked: 'Needs review',
      executing: 'Deleting',
    })[state] ?? state;

  async function refresh() {
    const current = requests.begin();
    try {
      const result = await api<{ items: Item[] }>('/admin/retention');
      if (current()) {
        items = result.items;
        loaded = true;
        error = '';
      }
    } catch (failure) {
      if (current()) error = String(failure);
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
      await refresh();
    } catch (failure) {
      error = String(failure);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    if (view === 'activity') return;
    untrack(() => void refresh());
    const timer = setInterval(() => {
      if (!busy) void refresh();
    }, 30_000);
    return () => {
      clearInterval(timer);
      requests.invalidate();
    };
  });
</script>

<FormField class="settings-wide m-0">
  Audit view
  <select
    aria-label="Audit view"
    class={formControlClass}
    bind:value={view}
    disabled={busy}
  >
    <option value="activity">Administrative activity</option>
    <option value="scheduled">Scheduled deletions</option>
    <option value="deletions">Deletion history</option>
  </select>
</FormField>
{#if view === 'activity'}
  <History {user} audit />
{:else}
  {#if error}<p class="settings-wide text-danger" role="alert">{error}</p>{/if}
  <Panel
    class="settings-wide grid gap-4"
    aria-label={scheduled ? 'Scheduled deletions' : 'Deletion history'}
  >
    <h2 class="m-0">
      {scheduled ? 'Scheduled deletions' : 'Deletion history'}
      <span class="text-sm font-normal text-muted">{visible.length}</span>
    </h2>
    {#if !loaded}
      <p class="m-0 text-sm text-muted" role="status">
        {error ? 'Deletions could not be loaded.' : 'Loading deletions…'}
      </p>
    {:else}
      {#each visible as item (item.id)}
        <article
          class="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4"
        >
          <div class="min-w-0 flex-1 basis-60">
            <strong class="break-words">{item.title}</strong>
            <p class="my-1 text-xs text-muted">
              {reason(item)} · Scheduled for {date(item.due_at)}
            </p>
            {#if item.error}<p class="my-1 break-words text-xs text-muted">
                {item.error}
              </p>{/if}
          </div>
          {#if scheduled}
            <div class="flex flex-wrap gap-2">
              <Button
                variant="secondary"
                size="form"
                disabled={busy}
                onclick={() => act(item, 'keep')}>Keep</Button
              >
              <Button
                variant="danger"
                size="form"
                disabled={busy}
                onclick={() => act(item, 'delete')}>Delete now</Button
              >
            </div>
          {:else}
            <span class="text-xs text-muted">{stateLabel(item.state)}</span>
          {/if}
        </article>
      {:else}
        <p class="m-0 text-sm text-muted">
          {scheduled ? 'No scheduled deletions.' : 'No deletion history.'}
        </p>
      {/each}
    {/if}
  </Panel>
{/if}
