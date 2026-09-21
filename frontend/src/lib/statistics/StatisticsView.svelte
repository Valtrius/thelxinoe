<script lang="ts">
  import { eyebrowTextClass } from '../ui/styles';
  import { sources, completionDetails, statisticsDonut } from './helpers';
  import { statisticsPlatformMetrics } from './helpers';
  import { LatestRequest } from '../providers/latest-request';
  import { onMount, untrack } from 'svelte';
  import { SvelteURLSearchParams } from 'svelte/reactivity';
  import {
    CalendarDays,
    CircleHelp,
    Clock3,
    Eye,
    LoaderCircle,
    RefreshCw,
    Timer,
    X,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { formatWatchDuration, watchedCompletionPercent } from './helpers';
  import type {
    StatisticsOverview,
    StatisticsPlatform,
    StatisticsRange,
  } from './types';
  import PlatformIcon from './StatisticsIcon.svelte';
  import Button from '../ui/Button.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import ViewingRhythmChart from './ViewingRhythmChart.svelte';
  import WatchTimeChart from './WatchTimeChart.svelte';
  import History from '../History.svelte';

  let { user } = $props<{ user: User }>();
  let scope = $state('mine');
  let people = $state<User[]>([]);

  const ranges: { value: StatisticsRange; label: string }[] = [
    { value: '7d', label: '7D' },
    { value: '30d', label: '30D' },
    { value: '90d', label: '90D' },
    { value: 'all', label: 'All' },
  ];
  const platforms: { value: StatisticsPlatform; label: string }[] = [
    { value: 'all', label: 'All' },
    ...sources,
  ];
  const panelClass = 'panel min-w-0 border border-line bg-surface p-4';
  const timezone = $derived(user.timezone);

  let range = $state<StatisticsRange>('30d');
  let platform = $state<StatisticsPlatform>('all');
  let overview = $state<StatisticsOverview | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let measureDialog = $state<HTMLDialogElement | null>(null);
  const requests = new LatestRequest();
  const channelListKey = $derived(`${range}:${platform}:${scope}:${user.id}`);

  const platformMetrics = $derived(
    statisticsPlatformMetrics(overview, platform),
  );
  const dominantPlatform = $derived(
    sources
      .map((source) => ({
        name: source.label,
        seconds: platformMetrics[source.value].seconds,
      }))
      .reduce((largest, item) =>
        item.seconds > largest.seconds ? item : largest,
      ).name,
  );
  const completion = $derived(completionDetails(overview, platform));
  const completionPercent = $derived(
    overview
      ? watchedCompletionPercent(completion.started, completion.watched)
      : 0,
  );
  const contentMixTotal = $derived(
    completion.items.reduce((sum, item) => sum + item.value, 0),
  );
  const highestChannelSeconds = $derived(
    Math.max(
      0,
      ...(overview?.topChannels.map((item) => item.activeSeconds) ?? []),
    ),
  );
  const content = $derived.by(contentSummary);
  const topLabel = $derived(
    platform === 'movies'
      ? 'TOP FILMS'
      : platform === 'shows'
        ? 'TOP SHOWS'
        : platform === 'music'
          ? 'TOP ARTISTS'
          : platform === 'all' &&
              sources
                .slice(3)
                .some((source) => platformMetrics[source.value].seconds > 0)
            ? 'TOP CHANNELS & TITLES'
            : 'TOP CHANNELS',
  );

  $effect(() => {
    const selection = [range, platform, scope, user.id, user.role, timezone];
    void selection;
    untrack(() => void loadOverview());
  });

  onMount(() => {
    const interval = window.setInterval(() => {
      if (!document.hidden) void loadOverview(true);
    }, 30_000);
    return () => {
      window.clearInterval(interval);
      requests.invalidate();
    };
  });

  async function loadOverview(
    background = false,
    requestedRange = range,
    requestedPlatform = platform,
  ) {
    const current = requests.begin();
    if (background) refreshing = true;
    else {
      loading = true;
      overview = null;
    }
    error = null;
    try {
      const query = new SvelteURLSearchParams({
        range: requestedRange,
        platform: requestedPlatform,
      });
      const administrative = user.role === 'admin' && scope !== 'mine';
      if (administrative && scope !== 'all') query.set('user', scope);
      const [loaded, users] = await Promise.all([
        api<StatisticsOverview>(
          `${administrative ? '/admin' : '/me'}/statistics?${query}`,
        ),
        user.role === 'admin'
          ? api<{ items: User[] }>('/users')
          : Promise.resolve(null),
      ]);
      if (current()) {
        overview = loaded;
        people = users?.items ?? [];
      }
    } catch (caught) {
      if (current())
        error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      if (current()) {
        loading = false;
        refreshing = false;
      }
    }
  }

  function contentSummary() {
    if (!overview) return { label: 'Content watched', value: '—', note: '' };
    if (platform === 'twitch' || platform === 'kick') {
      return {
        label: 'Channels watched',
        value: String(
          platform === 'twitch'
            ? overview.twitchChannelsWatched
            : overview.kickChannelsWatched,
        ),
        note: 'With active playback in this period',
      };
    }
    const streamNotes =
      platform === 'all'
        ? [
            overview.twitchChannelsWatched > 0
              ? `${overview.twitchChannelsWatched} Twitch channels`
              : '',
            overview.kickChannelsWatched > 0
              ? `${overview.kickChannelsWatched} Kick channels`
              : '',
          ].filter(Boolean)
        : [];
    const streamNote =
      streamNotes.length > 0 ? ` · ${streamNotes.join(' · ')}` : '';
    return {
      label:
        platform === 'music'
          ? 'Tracks completed'
          : completion.label === 'items'
            ? 'Content completed'
            : `${completion.label[0].toUpperCase()}${completion.label.slice(1)} watched`,
      value: String(completion.watched),
      note:
        completion.started > 0
          ? `${completionPercent}% of ${completion.started} started${streamNote}`
          : `No ${completion.label} started${streamNote}`,
    };
  }

  function trackingDate(value?: string | null) {
    if (!value) return null;
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return null;
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeZone: timezone,
    }).format(date);
  }
</script>

<div class="statistics grid min-w-0 grid-cols-1 gap-3">
  {#if user.role === 'admin'}
    <div
      data-sidebar-resize="xy"
      class="flex flex-wrap items-end justify-between gap-3"
    >
      <label class="m-0 min-w-52 max-w-full"
        >Statistics for
        <select bind:value={scope} aria-label="Statistics user">
          <option value="mine">My statistics</option>
          <option value="all">All users</option>
          {#each people as person (person.id)}<option value={person.id}
              >{person.username}</option
            >{/each}
        </select>
      </label>
      <span class="text-[0.62rem] text-muted">Times shown in {timezone}</span>
    </div>
  {/if}
  <div class="flex min-w-0 flex-wrap items-center gap-2">
    <div data-sidebar-resize="xy" class="flex">
      <ExclusiveChoiceGroup
        choices={ranges}
        value={range}
        ariaLabel="Statistics range"
        onChange={(nextRange) => (range = nextRange)}
      />
    </div>

    <Button
      data-sidebar-resize="xy"
      size="sm"
      variant="secondary"
      onclick={() => measureDialog?.showModal()}
      ><CircleHelp class="size-3.5" />How this is measured</Button
    >

    <div data-sidebar-resize="xy" class="ml-auto max-w-full overflow-x-auto">
      <ExclusiveChoiceGroup
        choices={platforms}
        value={platform}
        ariaLabel="Statistics platform"
        onChange={(nextPlatform) => (platform = nextPlatform)}
      />
    </div>
    <Button
      data-sidebar-resize="xy"
      size="icon"
      variant="ghost"
      aria-label="Refresh statistics"
      title="Refresh statistics"
      disabled={refreshing}
      onclick={() => void loadOverview(true)}
      ><RefreshCw
        class={`size-4 ${refreshing ? 'animate-spin' : ''}`}
      /></Button
    >
  </div>

  {#if error}
    <section
      data-sidebar-resize="xy"
      class="panel border border-danger bg-surface p-4"
    >
      <p class={[eyebrowTextClass, 'm-0 text-danger']}>STATISTICS / ERROR</p>
      <div class="mt-2 flex flex-wrap items-center justify-between gap-3">
        <p class="text-sm text-muted" role="alert">{error}</p>
        <Button
          size="sm"
          variant="secondary"
          onclick={() => void loadOverview()}>Retry</Button
        >
      </div>
    </section>
  {/if}

  {#if loading && !overview}
    <section
      data-sidebar-resize="xy"
      class="panel grid min-h-72 place-items-center border border-line bg-surface"
    >
      <div class="text-center">
        <LoaderCircle class="mx-auto size-6 animate-spin text-accent" />
        <p class={[eyebrowTextClass, 'm-0 mt-4']}>LOCAL / STATISTICS</p>
        <p class="mt-2 text-sm text-muted">Reading viewing history.</p>
      </div>
    </section>
  {:else if overview}
    {#if overview.totalActiveSeconds === 0}
      <section
        data-sidebar-resize="xy"
        class="panel border border-line-strong bg-accent-soft px-4 py-3"
      >
        <p class="text-xs font-semibold text-accent">
          No active playback in this view yet.
        </p>
        <p class="mt-1 text-[0.66rem] leading-5 text-muted">
          Play a video, stream, film, episode or music track to fill the time
          charts. Recorded progress and completion data appear below.
        </p>
      </section>
    {/if}

    {#if overview.estimatedActiveSeconds > 0}
      <p
        data-sidebar-resize="xy"
        class="m-0 border border-line bg-surface-soft px-4 py-3 text-[0.66rem] text-muted"
      >
        {formatWatchDuration(overview.estimatedActiveSeconds)} from earlier sessions
        is included. Its day and hour placement is approximate; new activity is recorded
        by minute.
      </p>
    {/if}

    <section
      class="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-4"
      aria-label="Statistics summary"
    >
      <article
        data-sidebar-resize="xy"
        class="panel min-w-0 border border-line bg-surface p-4"
      >
        <div class="flex items-center justify-between gap-3">
          <span
            class="text-[0.62rem] font-semibold tracking-[0.09em] text-muted uppercase"
            >Watch time</span
          >
          <Timer class="size-4 text-muted" />
        </div>
        <strong
          class="mt-3 block text-2xl leading-none font-medium tracking-[-0.055em]"
          >{formatWatchDuration(overview.totalActiveSeconds)}</strong
        >
        <span class="mt-2 block truncate text-[0.64rem] text-muted"
          >Active playback across devices</span
        >
      </article>
      <article
        data-sidebar-resize="xy"
        class="panel min-w-0 border border-line bg-surface p-4"
      >
        <div class="flex items-center justify-between gap-3">
          <span
            class="text-[0.62rem] font-semibold tracking-[0.09em] text-muted uppercase"
            >Watched days</span
          >
          <CalendarDays class="size-4 text-muted" />
        </div>
        <strong
          class="mt-3 block text-2xl leading-none font-medium tracking-[-0.055em]"
          >{overview.activeDays}{overview.periodDays > 0
            ? ` / ${overview.periodDays}`
            : ''}</strong
        >
        <span class="mt-2 block truncate text-[0.64rem] text-muted"
          >{trackingDate(overview.trackingStartedAt)
            ? `Tracking since ${trackingDate(overview.trackingStartedAt)}`
            : 'No tracked playback yet'}</span
        >
      </article>
      <article
        data-sidebar-resize="xy"
        class="panel min-w-0 border border-line bg-surface p-4"
      >
        <div class="flex items-center justify-between gap-3">
          <span
            class="text-[0.62rem] font-semibold tracking-[0.09em] text-muted uppercase"
            >Per watched day</span
          >
          <Clock3 class="size-4 text-muted" />
        </div>
        <strong
          class="mt-3 block text-2xl leading-none font-medium tracking-[-0.055em]"
          >{formatWatchDuration(overview.averageActiveSecondsPerDay)}</strong
        >
        <span class="mt-2 block truncate text-[0.64rem] text-muted"
          >Across days with playback</span
        >
      </article>
      <article
        data-sidebar-resize="xy"
        class="panel min-w-0 border border-line bg-surface p-4"
      >
        <div class="flex items-center justify-between gap-3">
          <span
            class="text-[0.62rem] font-semibold tracking-[0.09em] text-muted uppercase"
            >{content.label}</span
          >
          <Eye class="size-4 text-muted" />
        </div>
        <strong
          class="mt-3 block text-2xl leading-none font-medium tracking-[-0.055em]"
          >{content.value}</strong
        >
        <span class="mt-2 block truncate text-[0.64rem] text-muted"
          >{content.note}</span
        >
      </article>
    </section>

    <WatchTimeChart
      points={overview.activity}
      {platform}
      interval={overview.interval}
    />

    <section
      class="grid min-w-0 gap-3 xl:grid-cols-[minmax(16rem,0.7fr)_minmax(0,1.8fr)]"
    >
      <article data-sidebar-resize="xy" class={panelClass}>
        <header class="border-b border-line pb-3">
          <h2 class={[eyebrowTextClass, 'm-0']}>02 / PLATFORM SPLIT</h2>
        </header>
        <div
          class="grid min-h-72 place-items-center gap-5 py-4 sm:grid-cols-[minmax(8rem,0.8fr)_minmax(9rem,1fr)] xl:grid-cols-1 2xl:grid-cols-[minmax(8rem,0.8fr)_minmax(9rem,1fr)]"
        >
          <div
            class="relative aspect-square w-full max-w-40 rounded-full"
            style:background={statisticsDonut(platformMetrics)}
            role="img"
            aria-label={sources
              .map(
                (source) =>
                  `${Math.round(platformMetrics[source.value].share)} percent ${source.label}`,
              )
              .join(', ')}
          >
            <div
              class="absolute inset-[22%] grid place-items-center rounded-full border border-line bg-surface-strong text-center"
            >
              <div>
                <strong class="block text-lg font-semibold tracking-[-0.04em]"
                  >{formatWatchDuration(overview.totalActiveSeconds)}</strong
                ><span
                  class="mt-0.5 block text-[0.52rem] tracking-[0.08em] text-muted uppercase"
                  >Total</span
                >
              </div>
            </div>
          </div>
          <div class="grid w-full gap-4">
            {#each sources as source, index (source.value)}
              {#if index < 3 || platformMetrics[source.value].seconds > 0 || platform === source.value}
                <div>
                  <div
                    class="flex items-center justify-between gap-3 text-[0.66rem]"
                  >
                    <span class="text-muted">{source.label}</span>
                    <strong class="font-semibold"
                      >{formatWatchDuration(
                        platformMetrics[source.value].seconds,
                      )}</strong
                    >
                  </div>
                  <div class="mt-1.5 h-1 bg-surface-soft">
                    <i
                      class="block h-full"
                      style:background={source.color}
                      style:width={`${platformMetrics[source.value].share}%`}
                    ></i>
                  </div>
                </div>
              {/if}
            {/each}
            <p
              class="border-l-2 border-accent bg-accent-soft p-2.5 text-[0.64rem] leading-5 text-muted"
            >
              {overview.totalActiveSeconds > 0
                ? `${dominantPlatform} has the largest share of tracked playback in this period.`
                : 'No tracked playback in this period.'}
            </p>
          </div>
        </div>
      </article>

      <ViewingRhythmChart points={overview.rhythm} {platform} />
    </section>

    <section
      class="grid min-w-0 gap-3 xl:grid-cols-[minmax(0,1.35fr)_minmax(18rem,0.9fr)]"
    >
      <article data-sidebar-resize="xy" class={panelClass}>
        <header
          class="flex items-start justify-between gap-4 border-b border-line pb-3"
        >
          <div>
            <h2 id="top-channels-title" class={[eyebrowTextClass, 'm-0']}>
              04 / {topLabel}
            </h2>
          </div>
          <span class="text-[0.58rem] tracking-[0.08em] text-muted uppercase"
            >{platform === 'all' ? 'All platforms' : platform}</span
          >
        </header>
        {#if overview.topChannels.length > 0}
          {#key channelListKey}
            <div
              class="mt-2 max-h-65 [scrollbar-color:var(--line-strong)_transparent] [scrollbar-gutter:stable] overflow-y-auto overscroll-contain focus-visible:outline-1 focus-visible:outline-offset-2 focus-visible:outline-accent"
              role="region"
              aria-labelledby="top-channels-title"
            >
              <ol class="m-0 list-none p-0">
                {#each overview.topChannels as channel, index (channel.platform + channel.id)}
                  <li
                    class="grid h-13 min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_4rem] items-center gap-2 border-b border-line bg-transparent px-2 py-2.5 last:border-b-0 sm:grid-cols-[2rem_minmax(8rem,1.4fr)_minmax(5rem,1fr)_4rem] sm:gap-3"
                  >
                    <span
                      class="font-mono text-[0.64rem] text-muted tabular-nums"
                      aria-hidden="true">{index + 1}</span
                    >
                    <div class="flex min-w-0 items-center gap-2.5">
                      <span
                        class="grid size-8 shrink-0 place-items-center border border-line bg-surface-soft"
                        ><PlatformIcon
                          platform={channel.platform}
                          class="size-4"
                        /></span
                      >
                      <div class="min-w-0">
                        <strong
                          class="block truncate text-[0.69rem] font-semibold"
                          >{channel.name}</strong
                        ><span
                          class="mt-0.5 block truncate text-[0.56rem] text-muted"
                          >{channel.platform === 'youtube' ||
                          channel.platform === 'shows' ||
                          channel.platform === 'music'
                            ? `${channel.contentCount} ${channel.platform === 'music' ? 'tracks' : channel.platform === 'shows' ? 'episodes' : 'videos'}`
                            : `Watched on ${channel.watchedDays} days`}</span
                        >
                      </div>
                    </div>
                    <div class="hidden h-1 bg-surface-soft sm:block">
                      <i
                        class="block h-full bg-accent"
                        style={`width: ${highestChannelSeconds > 0 ? (channel.activeSeconds / highestChannelSeconds) * 100 : 0}%`}
                      ></i>
                    </div>
                    <span class="text-right text-[0.64rem] text-muted"
                      >{formatWatchDuration(channel.activeSeconds)}</span
                    >
                  </li>
                {/each}
              </ol>
            </div>
          {/key}
        {:else}
          <p class="grid min-h-48 place-items-center text-sm text-muted">
            No watch time in this period.
          </p>
        {/if}
      </article>

      <article data-sidebar-resize="xy" class={panelClass}>
        <header class="border-b border-line pb-3">
          <h2 class={[eyebrowTextClass, 'm-0']}>
            05 / {platform === 'twitch'
              ? 'TWITCH HISTORY'
              : platform === 'kick'
                ? 'KICK HISTORY'
                : completion.title}
          </h2>
        </header>
        {#if platform === 'twitch' || platform === 'kick'}
          <div class="grid min-h-64 content-center gap-4">
            <div>
              <strong class="text-4xl font-medium tracking-[-0.06em]"
                >{platform === 'twitch'
                  ? overview.twitchChannelsWatched
                  : overview.kickChannelsWatched}</strong
              ><span class="ml-2 text-xs text-muted">channels watched</span>
            </div>
            <p class="text-[0.66rem] leading-5 text-muted">
              {platform === 'twitch'
                ? 'Channel names, stream titles and categories are saved while you watch, so history remains available after a stream ends.'
                : 'Kick watch time is grouped by channel slug, with the available title and category saved while you watch.'}
            </p>
          </div>
        {:else}
          <div class="grid min-h-64 content-center gap-5 py-3">
            <div
              class="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-4"
            >
              <div
                class="relative size-22 rounded-full"
                style={`background: conic-gradient(var(--success) 0 ${completionPercent}%, var(--surface-soft) ${completionPercent}% 100%)`}
              >
                <div
                  class="absolute inset-2 grid place-items-center rounded-full bg-surface-strong text-base font-semibold"
                >
                  {completionPercent}%
                </div>
              </div>
              <div>
                <strong class="block text-sm font-semibold"
                  >{completion.watched} of {completion.started}
                  started {completion.label}</strong
                >
                <p class="mt-1.5 text-[0.63rem] leading-5 text-muted">
                  Completed by reaching 90%, finishing playback, or marking it
                  watched.
                  {completion.note}
                </p>
              </div>
            </div>
            <div class="grid gap-2.5 text-[0.62rem]">
              {#each completion.items as item (item.label)}
                <div
                  class="grid grid-cols-[5rem_minmax(0,1fr)_2rem] items-center gap-2"
                >
                  <span class="text-muted">{item.label}</span>
                  <div class="h-1.5 bg-surface-soft">
                    <i
                      class="block h-full bg-accent opacity-60"
                      style={`width: ${contentMixTotal > 0 ? (item.value / contentMixTotal) * 100 : 0}%`}
                    ></i>
                  </div>
                  <strong class="text-right">{item.value}</strong>
                </div>
              {/each}
            </div>
          </div>
        {/if}
      </article>
    </section>
    {#if user.role === 'admin' && scope === 'all' && overview.users.length > 0}
      <section
        class={panelClass}
        aria-label="Watch time by user"
        data-sidebar-resize="xy"
      >
        <h2 class={[eyebrowTextClass, 'm-0']}>USERS / WATCH TIME</h2>
        <div class="mt-3 grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {#each overview.users as person (person.id)}
            <button
              type="button"
              class="flex items-center justify-between gap-3 border border-line bg-surface-soft px-3 py-2 text-xs hover:border-line-strong hover:text-accent"
              onclick={() => (scope = person.id)}
            >
              <span class="truncate">{person.username}</span><strong
                class="shrink-0"
                >{formatWatchDuration(person.activeSeconds)}</strong
              >
            </button>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
  <History {user} {scope} {range} {platform} />
</div>

<dialog
  bind:this={measureDialog}
  class="panel m-auto max-h-[calc(100vh-3rem)] w-[min(48rem,calc(100vw-2rem))] overflow-y-auto border border-line-strong bg-surface-strong p-0 text-foreground shadow-[0_28px_100px_rgba(0,0,0,0.7)] backdrop:bg-black/65 backdrop:backdrop-blur-sm"
  aria-labelledby="statistics-measure-title"
>
  <header
    class="sticky top-0 z-1 flex items-start justify-between gap-4 border-b border-line bg-surface-strong p-5"
  >
    <div>
      <p class={[eyebrowTextClass, 'm-0']}>DATA / YOUR PLAYBACK</p>
      <h2 id="statistics-measure-title" class="mt-1 text-lg tracking-[-0.03em]">
        How statistics are measured
      </h2>
    </div>
    <Button
      size="icon"
      variant="ghost"
      aria-label="Close"
      onclick={() => measureDialog?.close()}><X class="size-4" /></Button
    >
  </header>
  <div class="grid gap-3 p-5 md:grid-cols-2">
    <section class="panel border-t-2 border-success bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>ACTIVE TIME</p>
      <h3 class="mt-1 text-sm font-semibold">The player clock</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        Web, music and native players count active wall time. Pauses, buffering,
        seeks and leaving the app open add nothing. Compatibility clients use
        conservative estimates from their progress reports. Playback on multiple
        devices is added together.
      </p>
    </section>
    <section class="panel border-t-2 border-accent bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>SMALL ROLLUPS</p>
      <h3 class="mt-1 text-sm font-semibold">One row per minute</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        Time is added to minute buckets by platform and media item. This keeps
        long-term charts small and lets local-time days and hours stay accurate.
      </p>
    </section>
    <section class="panel border-t-2 border-[#ff747b] bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>YOUTUBE</p>
      <h3 class="mt-1 text-sm font-semibold">Progress and completion</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        Video, channel, duration, type, first-played, and completion data supply
        library and channel statistics.
      </p>
    </section>
    <section class="panel border-t-2 border-[#b9a4ff] bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>TWITCH</p>
      <h3 class="mt-1 text-sm font-semibold">Channel snapshots</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        New activity rows keep the channel name, stream title, and game that
        were current while you watched.
      </p>
    </section>
    <section class="panel border-t-2 border-[#53fc18] bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>KICK</p>
      <h3 class="mt-1 text-sm font-semibold">Tracked channel totals</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        New activity rows use the channel slug and keep the available stream
        title and category. Disconnecting an account keeps its history; deleting
        its data removes that user's statistics too.
      </p>
    </section>
    <section class="panel border-t-2 border-[#65bfff] bg-surface p-4">
      <p class={[eyebrowTextClass, 'm-0']}>FILMS / SHOWS / MUSIC</p>
      <h3 class="mt-1 text-sm font-semibold">Your library history</h3>
      <p class="mt-3 text-[0.68rem] leading-5 text-muted">
        Films, episodes, shows, tracks, artists and albums are tracked per user.
        YouTube completion follows videos first started in the selected period;
        library totals cover items played in that period. Completion uses the
        current watched flag. Times follow your display timezone ({timezone}).
        Admins can inspect one user or aggregate everyone.
      </p>
    </section>
  </div>
</dialog>
