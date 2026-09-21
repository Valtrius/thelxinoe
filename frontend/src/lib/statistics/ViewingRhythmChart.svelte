<script lang="ts">
  import { eyebrowTextClass } from '../ui/styles';
  import { visibleSources } from './helpers';
  import { chartTooltipPosition } from './helpers';
  import {
    formatWatchDuration,
    gridNavigationTarget,
    heatLevel,
    selectedStatisticsSeconds,
  } from './helpers';
  import type { GridNavigationKey } from './helpers';
  import type { StatisticsPlatform, StatisticsRhythmPoint } from './types';

  type Tooltip = {
    point: StatisticsRhythmPoint;
    left: number;
    top: number;
  };

  let {
    points,
    platform,
  }: {
    points: StatisticsRhythmPoint[];
    platform: StatisticsPlatform;
  } = $props();

  const weekdays = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const hours = Array.from({ length: 24 }, (_, hour) => hour);
  const heatLegendClasses = [
    'border-line bg-surface-soft',
    'border-[color-mix(in_srgb,var(--accent)_25%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_16%,var(--surface-soft))]',
    'border-[color-mix(in_srgb,var(--accent)_30%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_32%,var(--surface-soft))]',
    'border-[color-mix(in_srgb,var(--accent)_38%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_48%,var(--surface-soft))]',
    'border-[color-mix(in_srgb,var(--accent)_48%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_64%,var(--surface-soft))]',
    'border-accent bg-[color-mix(in_srgb,var(--accent)_84%,var(--surface-soft))]',
  ];
  let tooltip = $state<Tooltip | null>(null);
  let gridElement = $state<HTMLElement | null>(null);
  let activeCellIndex = $state(0);
  const pointMap = $derived(
    new Map(points.map((point) => [`${point.weekday}:${point.hour}`, point])),
  );
  const maximumSeconds = $derived(
    Math.max(
      0,
      ...points.map((point) => selectedStatisticsSeconds(point, platform)),
    ),
  );

  function pointFor(weekday: number, hour: number): StatisticsRhythmPoint {
    return (
      pointMap.get(`${weekday}:${hour}`) ?? {
        weekday,
        hour,
        youtubeSeconds: 0,
        twitchSeconds: 0,
        kickSeconds: 0,
        moviesSeconds: 0,
        showsSeconds: 0,
        musicSeconds: 0,
      }
    );
  }

  function total(point: StatisticsRhythmPoint) {
    return selectedStatisticsSeconds(point, platform);
  }

  function level(point: StatisticsRhythmPoint) {
    return heatLevel(total(point), maximumSeconds);
  }

  function hourLabel(hour: number) {
    return `${String(hour).padStart(2, '0')}:00`;
  }

  function showTooltip(
    point: StatisticsRhythmPoint,
    clientX: number,
    clientY: number,
  ) {
    tooltip = {
      point,
      ...chartTooltipPosition(clientX, clientY, window.innerWidth),
    };
  }

  function focusTooltip(event: FocusEvent, point: StatisticsRhythmPoint) {
    const cellIndex = Number(
      (event.currentTarget as HTMLElement).dataset.cellIndex,
    );
    if (Number.isInteger(cellIndex)) activeCellIndex = cellIndex;
    const bounds = (event.currentTarget as HTMLElement).getBoundingClientRect();
    showTooltip(point, bounds.left + bounds.width / 2, bounds.top);
  }

  function handleCellKeydown(event: KeyboardEvent, cellIndex: number) {
    if (
      ![
        'ArrowLeft',
        'ArrowRight',
        'ArrowUp',
        'ArrowDown',
        'Home',
        'End',
      ].includes(event.key)
    ) {
      return;
    }
    event.preventDefault();
    const targetIndex = gridNavigationTarget(
      cellIndex,
      event.key as GridNavigationKey,
      weekdays.length,
      hours.length,
    );
    activeCellIndex = targetIndex;
    gridElement
      ?.querySelector<HTMLButtonElement>(`[data-cell-index="${targetIndex}"]`)
      ?.focus();
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
      <h2 class={[eyebrowTextClass, 'm-0']}>03 / VIEWING RHYTHM</h2>
    </div>
    <span class="shrink-0 text-[0.58rem] tracking-[0.08em] text-muted uppercase"
      >Time by hour</span
    >
  </header>

  <div class="mt-4 overflow-x-auto pb-2">
    <div
      bind:this={gridElement}
      class="grid min-w-232 grid-cols-[2rem_repeat(24,minmax(1.25rem,1fr))] items-center gap-1.5"
      role="grid"
      aria-label="Watch time by weekday and hour"
    >
      <div class="contents" role="row">
        <span role="columnheader"></span>
        {#each hours as hour (hour)}
          <span
            class="text-center text-[0.48rem] text-muted"
            role="columnheader"
            >{hour % 3 === 0 ? String(hour).padStart(2, '0') : ''}</span
          >
        {/each}
      </div>
      {#each weekdays as day, weekday (day)}
        <div class="contents" role="row">
          <span class="text-right text-[0.52rem] text-muted" role="rowheader"
            >{day}</span
          >
          {#each hours as hour (hour)}
            {@const point = pointFor(weekday, hour)}
            {@const pointTotal = total(point)}
            {@const cellIndex = weekday * hours.length + hour}
            <button
              type="button"
              role="gridcell"
              tabindex={cellIndex === activeCellIndex ? 0 : -1}
              data-cell-index={cellIndex}
              class={`aspect-square min-h-5 border transition-[border-color,transform] hover:z-1 hover:scale-110 hover:border-accent focus-visible:z-1 focus-visible:scale-110 focus-visible:border-accent focus-visible:shadow-[0_0_0_1px_var(--accent)] focus-visible:outline-none ${hour === hours.length - 1 ? 'origin-right' : ''} ${
                level(point) === 0
                  ? 'border-line bg-surface-soft'
                  : level(point) === 1
                    ? 'border-[color-mix(in_srgb,var(--accent)_18%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_16%,var(--surface-soft))]'
                    : level(point) === 2
                      ? 'border-[color-mix(in_srgb,var(--accent)_28%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_30%,var(--surface-soft))]'
                      : level(point) === 3
                        ? 'border-[color-mix(in_srgb,var(--accent)_38%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_46%,var(--surface-soft))]'
                        : level(point) === 4
                          ? 'border-[color-mix(in_srgb,var(--accent)_52%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_64%,var(--surface-soft))]'
                          : 'border-accent bg-[color-mix(in_srgb,var(--accent)_84%,var(--surface-soft))]'
              }`}
              aria-label={`${day} ${hourLabel(hour)} to ${hourLabel((hour + 1) % 24)}, ${formatWatchDuration(pointTotal)} watched`}
              onkeydown={(event) => handleCellKeydown(event, cellIndex)}
              onpointerenter={(event) =>
                showTooltip(point, event.clientX, event.clientY)}
              onpointermove={(event) => {
                if (tooltip) showTooltip(point, event.clientX, event.clientY);
              }}
              onpointerleave={() => (tooltip = null)}
              onfocus={(event) => focusTooltip(event, point)}
              onblur={() => (tooltip = null)}
            ></button>
          {/each}
        </div>
      {/each}
    </div>
  </div>
  <div
    class="mt-2 flex items-center justify-between gap-3 text-[0.6rem] text-muted"
  >
    <span>Hover a square or use arrow keys for exact watch time.</span>
    <span class="flex items-center gap-1.5" aria-hidden="true"
      >Less
      {#each [0, 1, 2, 3, 4, 5] as heat (heat)}
        <i class={`size-2.5 border ${heatLegendClasses[heat]}`}></i>
      {/each}
      More</span
    >
  </div>
</section>

{#if tooltip}
  <div
    class="panel pointer-events-none fixed z-300 w-58 border border-line-strong bg-surface-strong p-3 shadow-[0_16px_42px_var(--shadow)]"
    style={`left: ${tooltip.left}px; top: ${tooltip.top}px`}
    role="tooltip"
  >
    <strong class="block text-[0.68rem] font-semibold"
      >{weekdays[tooltip.point.weekday]} · {hourLabel(
        tooltip.point.hour,
      )}–{hourLabel((tooltip.point.hour + 1) % 24)}</strong
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
