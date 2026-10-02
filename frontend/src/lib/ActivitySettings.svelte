<script lang="ts">
  import { RefreshCw } from '@lucide/svelte';
  import type { Job } from './api';
  import { attention, acknowledgeAttention } from './attention';
  import AttentionDot from './ui/AttentionDot.svelte';
  import Button from './ui/Button.svelte';
  import FormField from './ui/FormField.svelte';
  import Panel from './ui/Panel.svelte';
  import SectionHeading from './ui/SectionHeading.svelte';
  import { badgeClass, formControlClass, rowClass } from './ui/styles';

  let {
    jobs,
    checkpoint,
  }: {
    jobs: Job[];
    checkpoint: () => Promise<void>;
  } = $props();
  let state = $state<'all' | Job['state']>('all');
  const states: Record<Job['state'], string> = {
    running: 'Running',
    queued: 'Queued',
    complete: 'Completed',
    failed: 'Failed',
  };
  const visible = $derived(
    jobs.filter((job) => state === 'all' || job.state === state),
  );
</script>

<Panel class="settings-wide">
  <SectionHeading>
    <h2>Background jobs</h2>
    <Button size="form" variant="secondary" onclick={checkpoint}
      ><RefreshCw size={15} /> Run checkpoint</Button
    >
  </SectionHeading>
  <div class="my-5 flex flex-wrap items-end justify-between gap-3">
    <FormField class="mb-0"
      >Job state<select
        aria-label="Job state"
        class={formControlClass}
        bind:value={state}
      >
        <option value="all">All states</option>
        {#each Object.entries(states) as [value, label] (value)}<option {value}
            >{label}</option
          >{/each}
      </select></FormField
    >
    <span class="text-xs text-muted" role="status"
      >{visible.length} of {jobs.length} latest {jobs.length === 1
        ? 'job'
        : 'jobs'}</span
    >
  </div>
  {#each visible as job (job.id)}
    {@const entries = $attention.filter(
      (item) => item.target === 'jobs' && item.resource === job.id,
    )}
    <div class={rowClass}>
      <div class="flex-1">
        <strong>{job.kind}</strong>{#if job.error}<small>{job.error}</small
          >{/if}
      </div>
      <span class={badgeClass}>{states[job.state]}</span>
      <AttentionDot items={entries} />
      {#each entries as entry (entry.id)}<Button
          variant="secondary"
          size="sm"
          onclick={() => void acknowledgeAttention(entry)}
          >Dismiss failure</Button
        >{/each}
    </div>
  {:else}<p class="text-muted">
      {state === 'all'
        ? 'No background jobs yet.'
        : 'No matching jobs in the latest activity.'}
    </p>{/each}
</Panel>
