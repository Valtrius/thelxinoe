<script lang="ts">
  import { eyebrowTextClass } from '../ui/styles';
  import { visibleSources } from './helpers';
  import { chartTooltipPosition } from './helpers';
  import { formatWatchDuration, selectedStatisticsSeconds } from './helpers';
  import type { StatisticsActivityPoint, StatisticsPlatform } from './types';

  type Tooltip = {
    point: StatisticsActivityPoint;
    left: number;
    top: number;
  };

  let {
    points,
    platform,
    interval,
  }: {
    points: StatisticsActivityPoint[];
    platform: StatisticsPlatform;
    interval: 'day' | 'week' | 'month';
  } = $props();

  let tooltip = $state<Tooltip | null>(null);

  const maximumSeconds = $derived.by(() => {
    const maximum = Math.max(
      0,
      ...points.map((point) => selectedStatisticsSeconds(point, platform)),
    );
    return Math.max(3600, Math.ceil(maximum / 3600) * 3600);
  });
  const chartMinimumWidth = $derived(Math.max(680, points.length * 28));
  const labelStep = $derived(
    points.length > 32
      ? Math.ceil(points.length / 12)
      : points.length > 18
        ? 5
        : 1,
  );

  function total(point: StatisticsActivityPoint) {
    return selectedStatisticsSeconds(point, platform);
  }

  function periodLabel(periodStart: string, compact = false) {
    const value = new Date(`${periodStart}T12:00:00`);
    if (interval === 'month') {
      return new Intl.DateTimeFormat(undefined, {
        month: compact ? 'short' : 'long',
        year: compact ? '2-digit' : 'numeric',
      }).format(value);
    }
    if (interval === 'week') {
      return new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
      }).format(value);
    }
    return new Intl.DateTimeFormat(undefined, {
      month: compact ? undefined : 'short',
      day: 'numeric',
      weekday: compact ? undefined : 'short',
    }).format(value);
  }

  function showTooltip(
    point: StatisticsActivityPoint,
    clientX: number,
    clientY: number,
  ) {
    tooltip = {
      point,
      ...chartTooltipPosition(clientX, clientY, window.innerWidth),
    };
  }
</script>

<section
  data-sidebar-resize="xy"
  class="panel min-w-0 border border-line bg-surface p-4"
>
  <header
    class="flex min-w-0 items-start justify-between gap-4 border-b border-line pb-3"
  >
    <div class="min-w-0">
      <h2 class={[eyebrowTextClass, 'm-0']}>01 / DAILY ACTIVITY</h2>
    </div>
    <div class="flex flex-wrap justify-end gap-3 text-[0.6rem] text-muted">
      {#each visibleSources(platform) as source (source.value)}
        <span class="flex items-center gap-1.5"
          ><i class="size-1.5 rounded-full" style:background={source.color}
          ></i>{source.label}</span
        >
      {/each}
    </div>
  </header>

  <div class="mt-4 overflow-x-auto pb-1">
    <div
      class="relative h-76"
      style={`min-width: ${chartMinimumWidth}px`}
      role="img"
      aria-label="Watch time by {interval}"
    >
      <div
        class="absolute inset-x-0 top-0 bottom-7 text-[0.54rem] text-muted"
        aria-hidden="true"
      >
        {#each [1, 0.66, 0.33, 0] as ratio (ratio)}
          <div
            class="absolute inset-x-0 border-t border-line"
            style={`top: ${(1 - ratio) * 100}%`}
          >
            <span class="absolute -top-2.5 left-0 bg-surface pr-2"
              >{formatWatchDuration(maximumSeconds * ratio)}</span
            >
          </div>
        {/each}
      </div>

      <div class="absolute inset-x-9 top-0 bottom-7 flex items-end gap-1.5">
        {#each points as point, index (point.periodStart)}
          {@const pointTotal = total(point)}
          <div
            class="relative flex h-full min-w-0 flex-1 items-end justify-center"
          >
            <div
              class="relative flex w-[min(72%,1.35rem)] min-w-1 origin-bottom flex-col-reverse overflow-hidden border border-[color-mix(in_srgb,var(--foreground)_8%,transparent)] bg-surface-soft transition-[border-color,height,transform] hover:z-1 hover:scale-110 hover:border-accent"
              style={`height: ${pointTotal > 0 ? Math.max(1.5, (pointTotal / maximumSeconds) * 100) : 0}%`}
              role="img"
              aria-label={`${periodLabel(point.periodStart)}, ${formatWatchDuration(pointTotal)}`}
              onpointerenter={(event) =>
                showTooltip(point, event.clientX, event.clientY)}
              onpointermove={(event) => {
                if (tooltip) showTooltip(point, event.clientX, event.clientY);
              }}
              onpointerleave={() => (tooltip = null)}
            >
              {#if pointTotal > 0}{#each visibleSources(platform) as source (source.value)}
                  <i
                    class="block"
                    style:background={source.color}
                    style:height={(point[source.field] / pointTotal) * 100 +
                      '%'}
                  ></i>
                {/each}{/if}
            </div>
            {#if index % labelStep === 0 || index === points.length - 1}<span
                class="absolute top-[calc(100%+0.42rem)] left-1/2 -translate-x-1/2 text-[0.52rem] whitespace-nowrap text-muted"
                >{periodLabel(point.periodStart, true)}</span
              >{/if}
          </div>
        {/each}
      </div>
    </div>
  </div>
  <table class="sr-only">
    <caption>Exact watch time by {interval}</caption>
    <thead>
      <tr>
        <th scope="col">Period</th>
        {#if platform === 'all'}<th scope="col">Total</th>{/if}
        {#each visibleSources(platform) as source (source.value)}<th scope="col"
            >{source.label}</th
          >{/each}
      </tr>
    </thead>
    <tbody>
      {#each points as point (point.periodStart)}
        <tr>
          <th scope="row">{periodLabel(point.periodStart)}</th>
          {#if platform === 'all'}<td>{formatWatchDuration(total(point))}</td
            >{/if}
          {#each visibleSources(platform) as source (source.value)}<td
              >{formatWatchDuration(point[source.field])}</td
            >{/each}
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="mt-2 text-[0.62rem] text-muted">
    Counts active playback across your devices. Pauses, buffering, seeking and
    idle players do not add time.
  </p>
</section>

{#if tooltip}
  <div
    class="panel pointer-events-none fixed z-300 w-58 border border-line-strong bg-surface-strong p-3 shadow-[0_16px_42px_var(--shadow)]"
    style={`left: ${tooltip.left}px; top: ${tooltip.top}px`}
    role="tooltip"
  >
    <strong class="block text-[0.68rem] font-semibold"
      >{periodLabel(tooltip.point.periodStart)}</strong
    >
    <span class="mt-1.5 block text-xs font-semibold text-accent"
      >{formatWatchDuration(total(tooltip.point))} watched</span
    >
    <span class="mt-1 block text-[0.58rem] text-muted"
      >{visibleSources(platform)
        .map(
          (source) =>
            source.label +
            ' ' +
            formatWatchDuration(tooltip!.point[source.field]),
        )
        .join(' · ')}</span
    >
  </div>
{/if}
