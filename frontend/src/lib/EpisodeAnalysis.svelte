<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { twMerge } from 'tailwind-merge';
  import { api } from './api';
  import { display, formatTimestamp } from './display.svelte';
  import {
    analysisPageSize,
    analysisQuery,
    coverageSummary,
    filteredCoverage,
    hasMoreEpisodes,
    otherSegments,
    segmentTimes,
    type AnalysisFilters,
    type AnalysisPage,
    type AnalysisStatus,
  } from './episode-analysis';
  import { dateFormat } from './intl';
  import { LatestRequest } from './latest-request';
  import { routeHref, sectionRoute } from './navigation';
  import { RefreshTask } from './refresh-task';
  import { revisions } from './server-events.svelte';
  import { isServerUpdateInterruption } from './server-updates';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import Notice from './ui/Notice.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import {
    cardBlockClass,
    cardTableClass,
    formControlClass,
  } from './ui/styles';

  const collapsedShows = 10;
  const statuses: Record<
    AnalysisStatus,
    { label: string; tone: 'neutral' | 'ok' | 'warn' | 'bad' | 'info' }
  > = {
    found: { label: 'Found', tone: 'ok' },
    nothing: { label: 'Nothing found', tone: 'neutral' },
    failed: { label: 'Failed', tone: 'bad' },
    waiting: { label: 'Waiting', tone: 'warn' },
    running: { label: 'Analyzing', tone: 'info' },
  };

  let show = $state('');
  let status = $state<AnalysisFilters['status']>('');
  let result = $state<AnalysisPage | null>(null);
  let switching = $state(false);
  let error = $state('');
  let loadingMore = $state(false);
  let allShows = $state(false);
  let limit: number | undefined;

  const filters = $derived({ show, status });
  const query = $derived(analysisQuery(filters));
  const coverage = $derived(result && filteredCoverage(result, show));
  const shows = $derived(
    allShows
      ? (result?.shows ?? [])
      : (result?.shows.slice(0, collapsedShows) ?? []),
  );
  const statusChoices = $derived([
    { value: '' as const, label: 'All', count: coverage?.episodes },
    {
      value: 'nothing' as const,
      label: 'Nothing found',
      count: coverage?.nothing,
    },
    { value: 'failed' as const, label: 'Failed', count: coverage?.failed },
    { value: 'waiting' as const, label: 'Waiting', count: coverage?.waiting },
  ]);
  const hasMore = $derived(
    !!result && !switching && hasMoreEpisodes(result, filters),
  );

  function analyzed(seconds: number) {
    const thisYear =
      new Date(seconds * 1000).getFullYear() === new Date().getFullYear();
    return dateFormat('en', {
      year: thisYear ? undefined : 'numeric',
      month: 'short',
      day: 'numeric',
      hour: display.timeFormat === '12h' ? 'numeric' : '2-digit',
      minute: '2-digit',
      hourCycle: display.timeFormat === '12h' ? 'h12' : 'h23',
      timeZone: display.timezone,
    }).format(seconds * 1000);
  }

  const requests = new LatestRequest();
  async function load() {
    const latest = requests.begin();
    try {
      const search = analysisQuery(filters, limit);
      const page = await api<AnalysisPage>(
        search
          ? `/admin/segments/analysis?${search}`
          : '/admin/segments/analysis',
      );
      if (!latest()) return;
      result = page;
      switching = false;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    }
  }
  async function loadMore() {
    if (!result) return;
    limit = result.items.length + analysisPageSize;
    loadingMore = true;
    try {
      await reads.refresh();
    } finally {
      loadingMore = false;
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
          switching = true;
          limit = undefined;
          loadingMore = false;
        }
        loadedQuery = next;
      }
      void reads.refresh();
    });
  });
</script>

{#snippet times(label: string, ranges: string[])}
  <td
    class="px-3 py-2 tabular-nums compact:order-3 compact:max-w-full compact:border-0 compact:p-0 compact:text-muted"
  >
    <span class="mr-1 hidden compact:inline">{label}</span>
    {#each ranges as time, index (index)}
      <span class="whitespace-nowrap">{time}</span>{index < ranges.length - 1
        ? ', '
        : ''}
    {:else}
      —
    {/each}
  </td>
{/snippet}

<SettingsCard
  title="Analysis results"
  description={result ? coverageSummary(result.totals) : undefined}
>
  {#if error}
    <div class={cardBlockClass}>
      <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
    </div>
  {/if}
  {#if !result}
    {#if !error}
      <div class={cardBlockClass}>
        <ContentSkeleton label="Loading analysis results" />
      </div>
    {/if}
  {:else if result.shows.length}
    <table
      class={twMerge(
        cardTableClass,
        'text-left compact:[&_tbody_td]:px-2 compact:[&_tbody_td:first-child]:pl-4 compact:[&_thead_th]:px-2 compact:[&_thead_th:first-child]:pl-4',
      )}
    >
      <caption class="sr-only">Coverage by show</caption>
      <thead>
        <tr>
          <th scope="col">Show</th>
          <th scope="col" class="w-0 text-right">Episodes</th>
          <th scope="col" class="w-0 text-right">Intro</th>
          <th scope="col" class="w-0 text-right">Credits</th>
          <th scope="col" class="w-0 text-right">Nothing found</th>
        </tr>
      </thead>
      <tbody>
        {#each shows as item (item.id)}
          <tr class={[show === item.id && 'bg-accent-soft']}>
            <td class="wrap-anywhere">
              <button
                type="button"
                class="text-left font-medium hover:text-accent"
                aria-pressed={show === item.id}
                onclick={() => (show = show === item.id ? '' : item.id)}
              >
                {item.title}
              </button>
            </td>
            <td class="text-right tabular-nums">{item.episodes}</td>
            <td class="text-right tabular-nums">{item.intro}</td>
            <td class="text-right tabular-nums">{item.credits}</td>
            <td class="text-right tabular-nums">{item.nothing}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if !allShows && result.shows.length > collapsedShows}
      <div class={cardBlockClass}>
        <Button
          variant="secondary"
          size="form"
          onclick={() => (allShows = true)}
          >Show all {result.shows.length} shows</Button
        >
      </div>
    {/if}
  {:else}
    <p class="m-0 {cardBlockClass} text-xs text-muted" role="status">
      No episodes to analyze yet.
    </p>
  {/if}
</SettingsCard>

{#if result?.shows.length}
  <SettingsCard title="Analyzed episodes">
    <div class="{cardBlockClass} flex flex-wrap items-center gap-x-4 gap-y-3">
      <div class="max-w-full overflow-x-auto">
        <ExclusiveChoiceGroup
          ariaLabel="Episode status"
          value={status}
          choices={statusChoices}
          onChange={(value) => (status = value)}
        />
      </div>
      <select
        aria-label="Show"
        class={twMerge(
          formControlClass,
          'h-8.5 w-52 max-w-full py-1.5 text-xs',
        )}
        bind:value={show}
      >
        <option value="">All shows</option>
        {#each result.shows as item (item.id)}
          <option value={item.id}>{item.title}</option>
        {/each}
      </select>
    </div>
    {#if switching}
      <div class={cardBlockClass}>
        <ContentSkeleton label="Loading analyzed episodes" />
      </div>
    {:else if result.items.length}
      <!-- Below the compact breakpoint the status stays beside the episode and the rest wraps below. -->
      <table class="w-full border-collapse text-left text-xs compact:block">
        <caption class="sr-only">Analyzed episodes</caption>
        <thead
          class="border-b border-line bg-surface-soft text-[0.62rem] text-muted compact:hidden"
        >
          <tr>
            <th scope="col" class="px-5.5 py-2 font-medium">Episode</th>
            <th scope="col" class="w-0 px-3 py-2 font-medium">Intro</th>
            <th scope="col" class="w-0 px-3 py-2 font-medium">Credits</th>
            <th scope="col" class="w-0 px-3 py-2 font-medium">Analyzed</th>
            <th scope="col" class="w-0 py-2 pr-5.5 pl-3 font-medium">Status</th>
          </tr>
        </thead>
        <tbody class="compact:block [&>tr:last-child>td]:border-b-0">
          {#each result.items as episode (`${episode.media_id}-${episode.file_id}`)}
            {@const others = otherSegments(episode.segments)}
            <tr
              class="compact:flex compact:flex-wrap compact:items-baseline compact:gap-x-3 compact:gap-y-1 compact:border-b compact:border-line compact:px-4 compact:py-2 compact:last:border-b-0"
            >
              <td
                class="px-5.5 py-2 wrap-anywhere compact:order-1 compact:basis-[calc(100%-8rem)] compact:border-0 compact:p-0"
              >
                <a
                  class="font-medium hover:text-accent"
                  href={routeHref({
                    ...sectionRoute('Shows'),
                    item: episode.media_id,
                  })}>{episode.title}</a
                >
                {#if others}<div class="mt-0.5 text-muted">{others}</div>{/if}
                {#if episode.error}<div class="mt-0.5 text-danger">
                    {episode.error}
                  </div>{/if}
              </td>
              {@render times('Intro', segmentTimes(episode.segments, 'Intro'))}
              {@render times(
                'Credits',
                segmentTimes(episode.segments, 'Credits'),
              )}
              <td
                class="px-3 py-2 whitespace-nowrap tabular-nums compact:order-3 compact:ml-auto compact:border-0 compact:p-0 compact:text-muted"
                title={episode.analyzed_at === null
                  ? undefined
                  : formatTimestamp(episode.analyzed_at)}
              >
                {episode.analyzed_at === null
                  ? '—'
                  : analyzed(episode.analyzed_at)}
              </td>
              <td
                class="py-2 pr-5.5 pl-3 whitespace-nowrap compact:order-2 compact:ml-auto compact:border-0 compact:p-0"
              >
                <StatusChip tone={statuses[episode.status].tone}
                  >{statuses[episode.status].label}</StatusChip
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="m-0 {cardBlockClass} text-xs text-muted" role="status">
        No episodes match these filters.
      </p>
    {/if}
    {#if hasMore}
      <div class={cardBlockClass}>
        <Button
          variant="secondary"
          size="form"
          disabled={loadingMore}
          onclick={() => void loadMore()}>Show more episodes</Button
        >
      </div>
    {/if}
  </SettingsCard>
{/if}
