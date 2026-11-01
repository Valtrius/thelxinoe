<script lang="ts">
  import { revisions } from './server-events.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api, type Job } from './api';
  import { LatestRequest } from './latest-request';
  import { isServerUpdateInterruption } from './server-updates';
  import Notice from './ui/Notice.svelte';
  import { attention, acknowledgeAttention } from './attention';
  import { activity } from './activity';
  import AttentionDot from './ui/AttentionDot.svelte';
  import Button from './ui/Button.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import SettingRow from './ui/SettingRow.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';

  let jobs = $state<Job[]>([]),
    loading = $state(true),
    error = $state('');
  const requests = new LatestRequest();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void revisions.settings;
    untrack(() => void load());
  });
  async function load() {
    const latest = requests.begin();
    try {
      const value = await api<{ items: Job[] }>('/admin/jobs');
      if (!latest()) return;
      jobs = value.items;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    } finally {
      if (latest()) loading = false;
    }
  }
  let shown = $state<'all' | Job['state']>('all');
  const states: Record<Job['state'], string> = {
    running: 'Running',
    queued: 'Queued',
    complete: 'Completed',
    failed: 'Failed',
  };
  const tones: Record<Job['state'], 'info' | 'warn' | 'ok' | 'bad'> = {
    running: 'info',
    queued: 'warn',
    complete: 'ok',
    failed: 'bad',
  };
  const pinned = $derived(
    new Set(
      $attention
        .filter((item) => item.target === 'jobs')
        .map((item) => item.resource),
    ),
  );
  // Failures stay on top until they are dismissed; the sort keeps each group's order.
  const visible = $derived(
    jobs
      .filter((job) => shown === 'all' || job.state === shown)
      .sort((a, b) => Number(pinned.has(b.id)) - Number(pinned.has(a.id))),
  );
</script>

<SettingsCard
  title="Background jobs"
  description="Library scans, synchronizations and maintenance tasks."
>
  {#snippet actions()}
    <select
      aria-label="Job state"
      class={twMerge(formControlClass, 'h-8.5 w-40 py-1.5 text-xs')}
      bind:value={shown}
    >
      <option value="all">All states</option>
      {#each Object.entries(states) as [value, label] (value)}<option {value}
          >{label}</option
        >{/each}
    </select>
  {/snippet}
  <p class="m-0 px-5.5 py-2.5 text-xs text-muted compact:px-4">
    <span role="status"
      >{visible.length} of {jobs.length} latest {jobs.length === 1
        ? 'job'
        : 'jobs'}</span
    >
  </p>
  {#if error}<div class={cardBlockClass}>
      <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
    </div>{/if}
  {#if loading}<div class={cardBlockClass}>
      <ContentSkeleton label="Loading background jobs" />
    </div>{:else}{#each visible as job (job.id)}
      {@const entries = $attention.filter(
        (item) => item.target === 'jobs' && item.resource === job.id,
      )}
      <SettingRow label={job.kind} description={job.error ?? undefined}>
        {#snippet badge()}
          <StatusChip tone={tones[job.state]}>{states[job.state]}</StatusChip>
          <AttentionDot
            items={entries}
            activity={$activity.filter(
              (item) => item.target === 'jobs' && item.resource === job.id,
            )}
          />
        {/snippet}
        {#each entries as entry (entry.id)}<Button
            variant="secondary"
            size="sm"
            onclick={() => void acknowledgeAttention(entry)}
            >Dismiss failure</Button
          >{/each}
      </SettingRow>
    {:else}<p class="m-0 {cardBlockClass} text-xs text-muted">
        {shown === 'all'
          ? 'No background jobs yet.'
          : 'No matching jobs in the latest activity.'}
      </p>{/each}{/if}
</SettingsCard>
