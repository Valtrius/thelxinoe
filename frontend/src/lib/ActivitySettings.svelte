<script lang="ts">
  import type { Job } from './api';
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

  let {
    jobs,
    loading = false,
  }: {
    jobs: Job[];
    loading?: boolean;
  } = $props();
  let state = $state<'all' | Job['state']>('all');
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
  const visible = $derived(
    jobs.filter((job) => state === 'all' || job.state === state),
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
      bind:value={state}
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
        {state === 'all'
          ? 'No background jobs yet.'
          : 'No matching jobs in the latest activity.'}
      </p>{/each}{/if}
</SettingsCard>
