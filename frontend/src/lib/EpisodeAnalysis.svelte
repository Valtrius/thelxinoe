<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { fade } from 'svelte/transition';
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
  import { getAnimationDuration } from './motion';
  import { routeHref, sectionRoute } from './navigation';
  import { pagePreparing } from './page-readiness';
  import { RefreshTask } from './refresh-task';
  import { revisions } from './server-events.svelte';
  import { isServerUpdateInterruption } from './server-updates';
  import Button from './ui/Button.svelte';
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import Notice from './ui/Notice.svelte';
  import { RowMotion } from './ui/row-motion.svelte';
  import RowMotionBody from './ui/RowMotionBody.svelte';
  import SettingsCard from './ui/SettingsCard.svelte';
  import StatusChip from './ui/StatusChip.svelte';
  import {
    cardBlockClass,
    cardTableClass,
    formControlClass,
    quietDisabledClass,
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

  const motion = new RowMotion();
  const coverageMotion = new RowMotion();
  let show = $state('');
  let status = $state<AnalysisFilters['status']>('');
  let result = $state<AnalysisPage | null>(null);
  // The filters `result` was read for, which `show` and `status` are ahead of during a read.
  let applied = $state<AnalysisFilters>({ show: '', status: '' });
  let error = $state('');
  let loadingMore = $state(false);
  let allShows = $state(false);
  let limit: number | undefined;
  // The filters `result` answers. A read of any others replaces the rows.
  let shownQuery: string | undefined;

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
  const hasMore = $derived(!!result && hasMoreEpisodes(result, applied));

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
  async function load(signal: AbortSignal) {
    const latest = requests.begin();
    const requested = filters;
    const requestedQuery = query;
    const replacing = requestedQuery !== shownQuery;
    try {
      const search = analysisQuery(requested, limit);
      const page = await api<AnalysisPage>(
        search
          ? `/admin/segments/analysis?${search}`
          : '/admin/segments/analysis',
        'GET',
        undefined,
        signal,
      );
      if (!latest()) return;
      const apply = () => {
        result = page;
        applied = requested;
        shownQuery = requestedQuery;
        error = '';
      };
      // One result feeds both tables, so each animates around the same update.
      const options = { live: !replacing && !loadingMore };
      await coverageMotion.commit(
        () => void motion.commit(apply, options),
        options,
      );
    } catch (caught) {
      if (!latest() || isServerUpdateInterruption(caught)) return;
      coverageMotion.release();
      await motion.commit(() => {
        // Rows that do not match the filters cannot stay on show.
        if (replacing && result) {
          result = { ...result, items: [] };
          shownQuery = undefined;
        }
        error = String(caught);
      });
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
  onMount(() => {
    coverageMotion.hold();
    return reads.start(false);
  });
  onDestroy(() => requests.invalidate());
  let loadedQuery: string | undefined;
  $effect(() => {
    void revisions.settings;
    const next = query;
    untrack(() => {
      if (loadedQuery === next) {
        void reads.refresh();
        return;
      }
      if (loadedQuery !== undefined) {
        requests.invalidate();
        motion.hold();
        limit = undefined;
        loadingMore = false;
      }
      loadedQuery = next;
      void reads.restart();
    });
  });
</script>

{#snippet times(label: string, ranges: string[])}
  <td
    class="px-3 py-2 tabular-nums table-stack:order-3 table-stack:max-w-full table-stack:border-0 table-stack:p-0 table-stack:text-muted"
  >
    <span class="mr-1 hidden table-stack:inline">{label}</span>
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
  <RowMotionBody
    motion={coverageMotion}
    loaded={!!result}
    class="divide-y divide-line"
  >
    {#if error}
      <div class={cardBlockClass} data-motion-row="error">
        <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
      </div>
    {/if}
    {#if !result}
      {#if coverageMotion.stale && !error}
        <div class={cardBlockClass}>
          <ContentSkeleton label="Loading analysis results" />
        </div>
      {/if}
    {:else if result.shows.length}
      <table
        class={twMerge(
          cardTableClass,
          'border-separate border-spacing-0 text-left table-stack:[&_th]:tracking-normal table-stack:[&_tbody_td]:px-1.5 table-stack:[&_tbody_td:first-child]:pl-5.5 table-stack:[&_thead_th]:px-1.5 table-stack:[&_thead_th:first-child]:pl-5.5 compact:[&_tbody_td:first-child]:pl-4 compact:[&_thead_th]:px-1.5 compact:[&_thead_th:first-child]:pl-4',
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
            <tr
              data-motion-row={`show:${item.id}`}
              class={[
                'transition-colors motion-reduce:transition-none',
                show === item.id && 'bg-accent-soft',
              ]}
            >
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
        <div class={cardBlockClass} data-motion-row="all-shows">
          <Button
            variant="secondary"
            size="form"
            onclick={() => void coverageMotion.commit(() => (allShows = true))}
            >Show all {result.shows.length} shows</Button
          >
        </div>
      {/if}
    {:else}
      <p
        class="m-0 {cardBlockClass} text-xs text-muted"
        role="status"
        data-motion-row="empty"
      >
        No episodes to analyze yet.
      </p>
    {/if}
  </RowMotionBody>
</SettingsCard>

{#if result?.shows.length}
  <div
    class="grid min-w-0"
    in:fade={{ duration: pagePreparing() ? 0 : getAnimationDuration() }}
  >
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
      <RowMotionBody {motion} class="divide-y divide-line">
        {#if result.items.length}
          <!-- Pinned columns stay put between filters. When they stop fitting, the status stays beside the episode and the rest wraps below. -->
          <table
            class="w-full table-fixed border-separate border-spacing-0 text-left text-xs table-stack:block"
          >
            <caption class="sr-only">Analyzed episodes</caption>
            <colgroup class="table-stack:hidden">
              <col />
              <col class="w-30" />
              <col class="w-30" />
              <col class="w-37" />
              <col class="w-37" />
            </colgroup>
            <thead
              class="border-b border-line bg-surface-soft text-[0.62rem] text-muted table-stack:hidden"
              data-motion-row="head"
            >
              <tr>
                <th scope="col" class="px-5.5 py-2 font-medium">Episode</th>
                <th scope="col" class="px-3 py-2 font-medium">Intro</th>
                <th scope="col" class="px-3 py-2 font-medium">Credits</th>
                <th scope="col" class="px-3 py-2 font-medium">Analyzed</th>
                <th scope="col" class="py-2 pr-5.5 pl-3 font-medium">Status</th>
              </tr>
            </thead>
            <tbody class="table-stack:block [&>tr:last-child>td]:border-b-0">
              {#each result.items as episode (`${episode.media_id}-${episode.file_id}`)}
                {@const others = otherSegments(episode.segments)}
                <tr
                  data-motion-row={`${episode.media_id}-${episode.file_id}`}
                  class="compact:px-4 table-stack:flex table-stack:flex-wrap table-stack:items-baseline table-stack:gap-x-3 table-stack:gap-y-1 table-stack:border-b table-stack:border-line table-stack:px-5.5 table-stack:py-2 table-stack:last:border-b-0"
                >
                  <td
                    class="px-5.5 py-2 wrap-anywhere table-stack:order-1 table-stack:basis-[calc(100%-8rem)] table-stack:border-0 table-stack:p-0"
                  >
                    <a
                      class="font-medium hover:text-accent"
                      href={routeHref({
                        ...sectionRoute('Shows'),
                        item: episode.media_id,
                      })}>{episode.title}</a
                    >
                    {#if others}<div class="mt-0.5 text-muted">
                        {others}
                      </div>{/if}
                    {#if episode.error}<div class="mt-0.5 text-danger">
                        {episode.error}
                      </div>{/if}
                  </td>
                  {@render times(
                    'Intro',
                    segmentTimes(episode.segments, 'Intro'),
                  )}
                  {@render times(
                    'Credits',
                    segmentTimes(episode.segments, 'Credits'),
                  )}
                  <td
                    class="px-3 py-2 whitespace-nowrap tabular-nums table-stack:order-3 table-stack:ml-auto table-stack:border-0 table-stack:p-0 table-stack:text-muted"
                    title={episode.analyzed_at === null
                      ? undefined
                      : formatTimestamp(episode.analyzed_at)}
                  >
                    {episode.analyzed_at === null
                      ? '—'
                      : analyzed(episode.analyzed_at)}
                  </td>
                  <td
                    class="py-2 pr-5.5 pl-3 whitespace-nowrap table-stack:order-2 table-stack:ml-auto table-stack:border-0 table-stack:p-0"
                  >
                    <StatusChip tone={statuses[episode.status].tone}
                      >{statuses[episode.status].label}</StatusChip
                    >
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {:else if !error}
          <p
            class="m-0 {cardBlockClass} text-xs text-muted"
            role="status"
            data-motion-row="empty"
          >
            No episodes match these filters.
          </p>
        {/if}
        {#if hasMore}
          <div class={cardBlockClass} data-motion-row="more">
            <Button
              variant="secondary"
              size="form"
              class={quietDisabledClass}
              disabled={loadingMore || motion.holding}
              onclick={() => void loadMore()}>Show more episodes</Button
            >
          </div>
        {/if}
      </RowMotionBody>
    </SettingsCard>
  </div>
{/if}
