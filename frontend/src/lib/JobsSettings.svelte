<script lang="ts">
  import { ChevronRight } from '@lucide/svelte';
  import { revisions } from './server-events.svelte';
  import { onDestroy, onMount, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { api, type Job, type JobPage } from './api';
  import { auditDays } from './audit-events';
  import { attention, acknowledgeAttention } from './attention';
  import { display, formatTimestamp } from './display.svelte';
  import { dateFormat } from './intl';
  import {
    formatJobDuration,
    hasOlderJobs,
    jobKinds,
    jobLabel,
    jobPageSize,
    jobRunTime,
    jobsQuery,
    jobTarget,
    jobTrigger,
  } from './jobs';
  import { LatestRequest } from './latest-request';
  import { RefreshTask } from './refresh-task';
  import { isServerUpdateInterruption } from './server-updates';
  import Notice from './ui/Notice.svelte';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import Switch from './ui/Switch.svelte';
  import { cardBlockClass, formControlClass } from './ui/styles';
  import { twMerge } from 'tailwind-merge';

  const ids = $props.id();
  const states: Record<Job['state'], string> = {
    running: 'Running',
    queued: 'Queued',
    failed: 'Failed',
    complete: 'Completed',
  };
  const tones: Record<Job['state'], 'info' | 'warn' | 'ok' | 'bad'> = {
    running: 'info',
    queued: 'warn',
    failed: 'bad',
    complete: 'ok',
  };
  const stateKeys = Object.keys(states) as Job['state'][];

  let kind = $state('');
  let shown = $state<Job['state'] | ''>('');
  let showAutomatic = $state(false);
  let result = $state<JobPage | null>(null);
  let totals = $state<JobPage['totals'] | null>(null);
  let error = $state('');
  let loadingOlder = $state(false);
  let limit: number | undefined;
  let since: number | undefined;
  let now = $state(Date.now() / 1000);
  const expanded = new SvelteSet<string>();

  const filters = $derived({ kind, state: shown, showAutomatic });
  const query = $derived(jobsQuery(filters));
  const stateChoices = $derived([
    {
      value: '' as const,
      label: 'All',
      count: totals
        ? stateKeys.reduce((sum, state) => sum + (totals?.[state] ?? 0), 0)
        : undefined,
    },
    ...stateKeys.map((state) => ({
      value: state,
      label: states[state],
      count: totals?.[state],
    })),
  ]);
  const failures = $derived(
    result?.pinned.filter((job) => job.state === 'failed') ?? [],
  );
  const inProgress = $derived(
    result?.pinned.filter((job) => job.state !== 'failed') ?? [],
  );
  const history = $derived.by(() => {
    const pinned = new Set(result?.pinned.map((job) => job.id));
    return result?.items.filter((job) => !pinned.has(job.id)) ?? [];
  });
  const days = $derived(auditDays(history, display.timezone));
  const hasOlder = $derived(!!result && hasOlderJobs(result, shown));
  const clock = $derived(
    dateFormat('en', {
      hour: display.timeFormat === '12h' ? 'numeric' : '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hourCycle: display.timeFormat === '12h' ? 'h12' : 'h23',
      timeZone: display.timezone,
    }),
  );

  const requests = new LatestRequest();
  async function load() {
    const latest = requests.begin();
    try {
      const search = jobsQuery(filters, { limit, since });
      const page = await api<JobPage>(
        search ? `/admin/jobs?${search}` : '/admin/jobs',
      );
      if (!latest()) return;
      result = page;
      totals = page.totals;
      // New jobs add to the older ones someone loaded instead of pushing them off the list.
      if (limit) since = page.items.at(-1)?.created_at ?? since;
      now = Date.now() / 1000;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    }
  }
  async function loadOlder() {
    if (!result) return;
    limit = result.items.length + jobPageSize;
    loadingOlder = true;
    try {
      await reads.refresh();
    } finally {
      loadingOlder = false;
    }
  }

  const reads = new RefreshTask(load, 30_000);
  onMount(() => reads.start(false));
  onDestroy(() => requests.invalidate());
  let loadedQuery: string | undefined;
  $effect(() => {
    void revisions.settings;
    const next = query;
    untrack(() => {
      if (loadedQuery !== next) {
        if (loadedQuery !== undefined) {
          requests.invalidate();
          result = null;
          limit = undefined;
          since = undefined;
          loadingOlder = false;
        }
        loadedQuery = next;
      }
      void reads.refresh();
    });
  });

  const running = $derived(inProgress.some((job) => job.state === 'running'));
  $effect(() => {
    if (!running) return;
    const timer = setInterval(() => (now = Date.now() / 1000), 1000);
    return () => clearInterval(timer);
  });

  function toggle(id: string) {
    if (expanded.has(id)) expanded.delete(id);
    else expanded.add(id);
  }
  async function dismiss(job: Job) {
    const entries = $attention.filter(
      (item) => item.target === 'jobs' && item.resource === job.id,
    );
    await Promise.all(entries.map((entry) => acknowledgeAttention(entry)));
    await reads.refresh();
  }
  function details(job: Job) {
    const started = job.started_at;
    const runTime = jobRunTime(job, now);
    const rows: [string, string | null][] = [
      ['Queued', formatTimestamp(job.created_at)],
      [
        'Scheduled for',
        started === null && job.available_at > now
          ? formatTimestamp(job.available_at)
          : null,
      ],
      ['Started', started === null ? null : formatTimestamp(started)],
      [
        'Waited',
        started === null
          ? null
          : formatJobDuration(Math.max(0, started - job.created_at)),
      ],
      [
        'Finished',
        job.completed_at === null ? null : formatTimestamp(job.completed_at),
      ],
      ['Ran for', runTime === null ? null : formatJobDuration(runTime)],
      ['Attempts', String(job.attempts)],
      ['Triggered by', jobTrigger(job)],
      ['Job ID', job.id],
    ];
    return rows.filter(([, value]) => value !== null);
  }
</script>

{#snippet heading(label: string, count: string)}
  <tbody class="compact:block">
    <tr class="compact:block">
      <th
        scope="colgroup"
        colspan="5"
        class="bg-surface-strong px-5.5 py-1.5 text-[0.7rem] font-semibold tracking-normal text-foreground normal-case compact:block compact:px-4"
      >
        {label}<span class="ml-2 font-normal text-muted">{count}</span>
      </th>
    </tr>
  </tbody>
{/snippet}

{#snippet row(job: Job, pinned: boolean)}
  {@const open = expanded.has(job.id)}
  {@const target = jobTarget(job)}
  {@const runTime = jobRunTime(job, now)}
  {@const moment = pinned
    ? (job.completed_at ?? job.started_at ?? job.created_at)
    : job.created_at}
  {@const dismissible =
    pinned &&
    $attention.some(
      (item) => item.target === 'jobs' && item.resource === job.id,
    )}
  <tbody class="border-b border-line compact:block">
    <tr
      class="compact:grid compact:grid-cols-[auto_minmax(0,1fr)_auto] compact:items-baseline compact:gap-x-3 compact:gap-y-1 compact:px-4 compact:py-2"
    >
      <td
        class="px-5.5 py-1.5 whitespace-nowrap tabular-nums compact:order-3 compact:border-0 compact:p-0 compact:text-muted"
        title={formatTimestamp(moment)}
      >
        {pinned ? formatTimestamp(moment) : clock.format(moment * 1000)}
      </td>
      <td
        class="px-3 py-1.5 compact:order-1 compact:col-span-2 compact:border-0 compact:p-0"
      >
        <button
          type="button"
          class="inline-flex items-center gap-1.5 text-left font-medium whitespace-nowrap hover:text-accent compact:whitespace-normal"
          aria-expanded={open}
          aria-controls={`${ids}-${job.id}`}
          title={job.kind}
          onclick={() => toggle(job.id)}
        >
          <ChevronRight
            size={13}
            aria-hidden="true"
            class={[
              'shrink-0 text-muted transition-transform motion-reduce:transition-none',
              open && 'rotate-90',
            ]}
          />
          {jobLabel(job)}
        </button>
      </td>
      <td
        class="px-3 py-1.5 wrap-anywhere compact:order-4 compact:border-0 compact:p-0 compact:text-muted"
      >
        {#if target}{target}{:else}<span class="text-muted italic">Removed</span
          >{/if}
      </td>
      <td
        class="px-3 py-1.5 whitespace-nowrap tabular-nums compact:order-5 compact:border-0 compact:p-0 compact:text-right compact:text-muted"
      >
        {runTime === null ? '—' : formatJobDuration(runTime)}
      </td>
      <td
        class="py-1.5 pr-5.5 pl-3 whitespace-nowrap compact:order-2 compact:border-0 compact:p-0 compact:text-right"
      >
        <span class="inline-flex items-center gap-2">
          <StatusChip tone={tones[job.state]}>{states[job.state]}</StatusChip>
          {#if dismissible}
            <Button
              variant="secondary"
              size="sm"
              onclick={() => void dismiss(job)}>Dismiss</Button
            >
          {/if}
        </span>
      </td>
    </tr>
    {#if job.error}
      <tr class="compact:block">
        <td
          colspan="5"
          class="px-5.5 pt-0 pb-2 text-danger wrap-anywhere compact:block compact:border-0 compact:px-4"
        >
          {job.error}
        </td>
      </tr>
    {/if}
    <tr class="compact:block" hidden={!open} id={`${ids}-${job.id}`}>
      <td
        colspan="5"
        class="border-t border-line bg-surface-soft px-5.5 py-3 compact:block compact:px-4"
      >
        {#if open}
          <dl
            class="m-0 grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-x-6 gap-y-2.5"
          >
            {#each details(job) as [label, value] (label)}
              <div class="min-w-0">
                <dt class="text-[0.62rem] text-muted">{label}</dt>
                <dd class="m-0 tabular-nums wrap-anywhere">{value}</dd>
              </div>
            {/each}
          </dl>
        {/if}
      </td>
    </tr>
  </tbody>
{/snippet}

<SettingsCard
  title="Background jobs"
  description={`Finished jobs are kept for 7 days. Times shown in ${display.timezone}.`}
>
  <div class="{cardBlockClass} flex flex-wrap items-center gap-x-4 gap-y-3">
    <div class="max-w-full overflow-x-auto">
      <ExclusiveChoiceGroup
        ariaLabel="Job state"
        value={shown}
        choices={stateChoices}
        onChange={(value) => (shown = value)}
      />
    </div>
    <select
      aria-label="Job type"
      class={twMerge(formControlClass, 'h-8.5 w-52 py-1.5 text-xs')}
      bind:value={kind}
    >
      <option value="">All types</option>
      {#each Object.entries(jobKinds) as [value, label] (value)}
        <option {value}>{label}</option>
      {/each}
    </select>
    <Switch size="sm" class="text-xs" bind:checked={showAutomatic}
      >Show automatic metadata refreshes</Switch
    >
  </div>
  {#if error}
    <div class={cardBlockClass}>
      <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
    </div>
  {/if}
  {#if !result}
    {#if !error}
      <div class={cardBlockClass}>
        <ContentSkeleton label="Loading background jobs" />
      </div>
    {/if}
  {:else if failures.length || inProgress.length || history.length}
    <!-- Below the compact breakpoint each job stacks into two lines. -->
    <table class="w-full border-collapse text-left text-xs compact:block">
      <caption class="sr-only">Background jobs</caption>
      <thead
        class="border-b border-line bg-surface-soft text-[0.62rem] text-muted compact:hidden"
      >
        <tr>
          <th scope="col" class="w-0 px-5.5 py-2 font-medium">Time</th>
          <th scope="col" class="w-0 px-3 py-2 font-medium">Job</th>
          <th scope="col" class="px-3 py-2 font-medium">Target</th>
          <th scope="col" class="w-0 px-3 py-2 font-medium">Duration</th>
          <th scope="col" class="w-0 py-2 pr-5.5 pl-3 font-medium">Status</th>
        </tr>
      </thead>
      {#if failures.length}
        {@render heading(
          'Needs attention',
          `${failures.length} ${failures.length === 1 ? 'job' : 'jobs'}`,
        )}
        {#each failures as job (job.id)}{@render row(job, true)}{/each}
      {/if}
      {#if inProgress.length}
        {@render heading(
          'In progress',
          `${inProgress.length} ${inProgress.length === 1 ? 'job' : 'jobs'}`,
        )}
        {#each inProgress as job (job.id)}{@render row(job, true)}{/each}
      {/if}
      {#each days as day, index (day.key)}
        {@const partial = index === days.length - 1 && hasOlder}
        {@render heading(
          day.label,
          `${day.rows.length}${partial ? '+' : ''} ${day.rows.length === 1 && !partial ? 'job' : 'jobs'}`,
        )}
        {#each day.rows as job (job.id)}{@render row(job, false)}{/each}
      {/each}
    </table>
  {:else if !error}
    <p class="m-0 {cardBlockClass} text-xs text-muted" role="status">
      {kind || shown || !showAutomatic
        ? 'No jobs match these filters.'
        : 'No background jobs yet.'}
    </p>
  {/if}
  {#if hasOlder}
    <div class={cardBlockClass}>
      <Button
        variant="secondary"
        size="form"
        disabled={loadingOlder}
        onclick={() => void loadOlder()}>Load older jobs</Button
      >
    </div>
  {/if}
</SettingsCard>
