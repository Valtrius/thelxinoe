<script lang="ts">
  import { tick, untrack } from 'svelte';
  import {
    CalendarClock,
    CheckCheck,
    Circle,
    CircleAlert,
    CircleCheck,
    CircleSlash,
    Contrast,
    Download,
    EyeOff,
    Zap,
  } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import ExclusiveChoiceGroup from '../ui/ExclusiveChoiceGroup.svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import { RowMotion } from '../ui/row-motion.svelte';
  import RowMotionBody from '../ui/RowMotionBody.svelte';
  import { dateFormat } from '../intl';
  import { showToast } from '../providers/toasts';
  import EpisodeRow from './EpisodeRow.svelte';
  import TitleMenu from './TitleMenu.svelte';
  import {
    episodeLabel,
    setFlag,
    setWatched,
    type Episode,
    type Season,
  } from './catalog';
  import {
    phase,
    seasonFiles,
    seasonToOpen,
    shown,
    summarize,
    type EpisodeFilter,
    type SeasonFiles,
    type SeasonSummary,
  } from './episodes';
  import { searchEpisodes } from './sonarr';

  let {
    seasons,
    show,
    series = null,
    next = null,
    admin,
    timezone,
    focusSeason,
    focusEpisode,
    play,
    changed,
  } = $props<{
    seasons: Season[];
    show: string;
    /** The library id or tmdb:/tvdb: reference the seasons load from. */
    series?: string | null;
    /** The episode Play on the title page starts. */
    next?: string | null;
    admin: boolean;
    timezone: string;
    focusSeason?: number | null;
    focusEpisode?: string | null;
    play: (episode: Episode, fileId?: string) => void;
    changed: () => void;
  }>();
  const ids = $props.id();
  const panelMotion = new RowMotion();
  const listMotion = new RowMotion();
  let selected = $state<number | null>(null),
    busy = $state<string | null>(null),
    // Watched toggles show immediately; the reloaded seasons replace them.
    toggled = $state<Record<string, boolean>>({}),
    filter = $state<EpisodeFilter>('all'),
    marking = $state(false),
    searching = $state(false),
    list = $state<HTMLElement>(),
    tabs = $state<HTMLElement>();
  // The linked season is chosen once; later refreshes keep the reader's own tab.
  let applied: number | null = null;
  const watched = (episode: Episode) =>
    (episode.id ? toggled[episode.id] : undefined) ?? episode.watched;
  const label = (episode: Episode) => episodeLabel(episode.title, show);
  const view = $derived(
    seasons.map((entry: Season) => ({
      ...entry,
      episodes: entry.episodes.map((episode) => ({
        ...episode,
        watched: watched(episode),
      })),
    })),
  );
  const summaries = $derived(
    new Map<number, SeasonSummary>(
      view.map((entry: Season) => [entry.number, summarize(entry)]),
    ),
  );
  const season = $derived<Season | undefined>(
    view.find((entry: Season) => entry.number === selected) ?? view[0],
  );
  const summary = $derived(season && summaries.get(season.number));
  const library = $derived(
    season?.episodes.filter((episode: Episode) => episode.id) ?? [],
  );
  const finished = $derived(
    library.length > 0 && library.every((episode: Episode) => episode.watched),
  );
  const wanted = $derived(
    season?.episodes.filter((episode: Episode) => phase(episode) === 'missing')
      .length ?? 0,
  );
  // A season without missing episodes has no Missing filter to stay on.
  const active = $derived<EpisodeFilter>(
    filter === 'missing' && !(admin && summary?.missing) ? 'all' : filter,
  );
  const episodes = $derived(
    season?.episodes.filter((episode: Episode) => shown(episode, active)) ?? [],
  );
  const year = (seconds: number) =>
    dateFormat('en', { timeZone: timezone, year: 'numeric' }).format(
      new Date(seconds * 1000),
    );
  const first = (entry: Season) =>
    entry.episodes
      .map((episode) => episode.air_date)
      .filter((date): date is number => date !== null)
      .sort((a, b) => a - b)[0];
  const fileBadges: Record<SeasonFiles, { icon: typeof Circle; tone: string }> =
    {
      complete: { icon: CircleCheck, tone: 'text-success' },
      ongoing: { icon: CalendarClock, tone: 'text-success' },
      upcoming: { icon: CalendarClock, tone: 'text-muted' },
      downloading: { icon: Download, tone: 'text-accent' },
      partial: { icon: Contrast, tone: 'text-warning' },
      empty: { icon: Circle, tone: 'text-muted' },
    };
  const tabTitle = (counts: SeasonSummary) =>
    [
      `${counts.available} of ${counts.total} ${counts.total === 1 ? 'episode' : 'episodes'} available`,
      counts.downloading && `${counts.downloading} downloading`,
      counts.upcoming && `${counts.upcoming} upcoming`,
      `${counts.watched} watched`,
    ]
      .filter(Boolean)
      .join(' · ');

  $effect(() => {
    const focus = focusSeason;
    void seasons;
    untrack(() => {
      toggled = {};
      if (
        focus != null &&
        focus !== applied &&
        seasons.some((s: Season) => s.number === focus)
      ) {
        selected = focus;
        applied = focus;
      } else if (!seasons.some((s: Season) => s.number === selected))
        selected = seasonToOpen(seasons);
    });
  });
  // The opened season stays inside the strip, which scrolls without moving the page.
  function revealTab() {
    const tab = tabs?.querySelector('[aria-selected="true"]');
    if (!tabs || !tab) return;
    const { left, right } = tab.getBoundingClientRect();
    const strip = tabs.getBoundingClientRect();
    // Whole pixels, since the browser rounds scrollLeft and could cut the tab.
    tabs.scrollLeft +=
      Math.floor(Math.min(0, left - strip.left)) +
      Math.ceil(Math.max(0, right - strip.right));
  }
  $effect(() => {
    void selected;
    void tick().then(revealTab);
  });
  $effect(() => {
    if (!tabs) return;
    const observer = new ResizeObserver(revealTab);
    observer.observe(tabs);
    return () => observer.disconnect();
  });
  $effect(() => {
    const episode = focusEpisode;
    if (!episode) return;
    void tick().then(() =>
      list
        ?.querySelector<HTMLElement>(`[data-episode="${CSS.escape(episode)}"]`)
        ?.scrollIntoView({ block: 'center' }),
    );
  });

  // "Up to here" also covers earlier seasons, which can wait for the reload.
  function showWatched(media: string, value: boolean, through = false) {
    const marked = view.flatMap((entry: Season) => {
      if (entry.id === media) return entry.episodes;
      const episode = entry.episodes.find((item) => item.id === media);
      if (!episode) return [];
      return through
        ? entry.episodes.filter((item) => item.number <= episode.number)
        : [episode];
    });
    void listMotion.commit(() => {
      for (const { id } of marked) if (id) toggled[id] = value;
    });
  }

  async function toggleWatched(episode: Episode) {
    if (!episode.id) return;
    busy = episode.id;
    const value = !watched(episode);
    try {
      await setFlag(episode.id, 'watched', value);
      showWatched(episode.id, value);
      changed();
    } catch (caught) {
      showToast({
        key: `watched:${episode.id}`,
        tone: 'error',
        title: label(episode),
        message: String(caught),
      });
    } finally {
      busy = null;
    }
  }

  async function markSeason() {
    if (!season?.id) return;
    marking = true;
    const media = season.id;
    const value = !finished;
    try {
      await setWatched(media, value);
      showWatched(media, value);
      changed();
    } catch (caught) {
      showToast({
        key: `watched:${season.id}`,
        tone: 'error',
        title: season.title,
        message: String(caught),
      });
    } finally {
      marking = false;
    }
  }

  async function searchMissing() {
    if (!season || !series) return;
    searching = true;
    try {
      const result = await searchEpisodes(series, {
        season: season.number,
        missing: true,
      });
      showToast({
        key: `search:${series}:${season.number}`,
        tone: 'success',
        title: season.title,
        message: `Sonarr is searching for ${result.episodes} missing ${result.episodes === 1 ? 'episode' : 'episodes'}.`,
      });
    } catch (caught) {
      showToast({
        key: `search:${series}:${season.number}`,
        tone: 'error',
        title: season.title,
        message: String(caught),
      });
    } finally {
      searching = false;
    }
  }

  function choose(number: number) {
    if (number === season?.number) return;
    void panelMotion.commit(
      () => {
        listMotion.settle();
        selected = number;
      },
      { dissolve: true },
    );
  }

  // Arrow keys move between season tabs, as in any tab list.
  function navigate(event: KeyboardEvent) {
    const order = seasons.map((s: Season) => s.number);
    const index = order.indexOf(season?.number);
    const target =
      event.key === 'ArrowRight'
        ? order[(index + 1) % order.length]
        : event.key === 'ArrowLeft'
          ? order[(index - 1 + order.length) % order.length]
          : event.key === 'Home'
            ? order[0]
            : event.key === 'End'
              ? order.at(-1)
              : undefined;
    if (target === undefined) return;
    event.preventDefault();
    choose(target);
    void tick().then(() =>
      tabs?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus(),
    );
  }
</script>

{#if view.length}
  <section class="mt-9" aria-label="Seasons">
    <div
      bind:this={tabs}
      class="flex min-w-0 gap-1 overflow-x-auto border-b border-line"
      role="tablist"
      aria-label="Seasons"
      tabindex="-1"
      onkeydown={navigate}
    >
      {#each view as entry (entry.number)}
        {@const counts = summaries.get(entry.number)!}
        {@const files = seasonFiles(counts)}
        {@const Icon = fileBadges[files].icon}
        <button
          id={`${ids}-tab-${entry.number}`}
          role="tab"
          aria-selected={entry.number === season?.number}
          aria-controls={`${ids}-panel`}
          tabindex={entry.number === season?.number ? 0 : -1}
          class={[
            'grid shrink-0 gap-2 border-b-2 px-4 pt-2 pb-2.5 text-xs font-semibold transition-colors hover:text-foreground',
            entry.number === season?.number
              ? 'border-accent text-accent'
              : 'border-transparent text-muted',
          ]}
          title={tabTitle(counts)}
          data-season-files={files}
          onclick={() => choose(entry.number)}
          ><span
            class={[
              'flex items-center gap-2',
              files === 'empty' &&
                entry.number !== season?.number &&
                'opacity-60',
            ]}
            >{entry.title}<span
              aria-hidden="true"
              class={[
                'inline-flex items-center gap-1 text-[10.5px] font-medium tabular-nums',
                fileBadges[files].tone,
              ]}
              ><Icon size={11} strokeWidth={2.5} />{counts.premiere
                ? year(counts.premiere)
                : `${counts.available}/${counts.total}`}</span
            ></span
          ><ProgressBar
            class="h-0.5 w-full bg-line text-muted"
            value={counts.total ? (counts.watched / counts.total) * 100 : 0}
          /></button
        >
      {/each}
    </div>
    {#if season && summary}
      <div
        id={`${ids}-panel`}
        role="tabpanel"
        aria-labelledby={`${ids}-tab-${season.number}`}
      >
        <RowMotionBody motion={panelMotion}>
          <div class="flex flex-wrap items-start justify-between gap-3 py-3.5">
            <div class="min-w-0">
              <p
                class="mb-0 flex flex-wrap items-baseline gap-x-2 text-[13px] font-semibold"
              >
                {season.title}<span class="text-[11.5px] font-normal text-muted"
                  >{[
                    first(season) && year(first(season)),
                    `${summary.total} ${summary.total === 1 ? 'episode' : 'episodes'}`,
                  ]
                    .filter(Boolean)
                    .join(' · ')}</span
                >
              </p>
              <p
                class="mt-2 mb-0 flex flex-wrap items-center gap-x-3.5 gap-y-1.5 text-[11px] text-muted"
              >
                <span class="inline-flex items-center gap-2"
                  ><span
                    ><span class="font-semibold text-foreground"
                      >{summary.watched}</span
                    >/{summary.total} watched</span
                  ><ProgressBar
                    class="h-0.75 w-15 bg-line"
                    value={summary.total
                      ? (summary.watched / summary.total) * 100
                      : 0}
                  /></span
                >{#if summary.available}<span
                    class="inline-flex items-center gap-1.5 text-success"
                    ><CircleCheck size={12} />{summary.available} available</span
                  >{/if}{#if summary.downloading}<span
                    class="inline-flex items-center gap-1.5 text-accent"
                    ><Download size={12} />{summary.downloading} downloading</span
                  >{/if}{#if summary.missing}<span
                    class="inline-flex items-center gap-1.5 text-warning"
                    ><CircleAlert size={12} />{summary.missing} missing</span
                  >{/if}{#if summary.unavailable}<span
                    class="inline-flex items-center gap-1.5"
                    ><CircleSlash size={12} />{summary.unavailable} not available</span
                  >{/if}{#if summary.upcoming}<span
                    class="inline-flex items-center gap-1.5"
                    ><CalendarClock size={12} />{summary.upcoming} upcoming</span
                  >{/if}
              </p>
            </div>
            <div class="flex shrink-0 flex-wrap items-center gap-1.5">
              {#if season.id && library.length}<Button
                  variant="secondary"
                  size="form"
                  loading={marking}
                  onclick={() => void markSeason()}
                  >{#if finished}<EyeOff size={15} />Mark season unwatched{:else}<CheckCheck
                      size={15}
                    />Mark season watched{/if}</Button
                >{/if}
              {#if admin && series && wanted}<Button
                  size="form"
                  loading={searching}
                  onclick={() => void searchMissing()}
                  ><Zap size={15} />Search missing</Button
                >{/if}
              {#key `${season.number}:${season.id}`}<TitleMenu
                  id={season.id}
                  kind="season"
                  title={season.title}
                  {admin}
                  sonarr={admin && series
                    ? { series, season: season.number }
                    : null}
                  {changed}
                  watchedChanged={showWatched}
                />{/key}
            </div>
          </div>
          {#if summary.total > 1}<div class="mb-1 flex items-center">
              <ExclusiveChoiceGroup
                ariaLabel="Show episodes"
                value={active}
                choices={[
                  { value: 'all', label: `All ${summary.total}` },
                  {
                    value: 'unwatched',
                    label: `Unwatched ${summary.total - summary.watched}`,
                  },
                  ...(admin && summary.missing
                    ? [
                        {
                          value: 'missing' as const,
                          label: `Missing ${summary.missing}`,
                        },
                      ]
                    : []),
                ]}
                onChange={(value) =>
                  void listMotion.commit(() => (filter = value))}
              />
            </div>{/if}
          <RowMotionBody motion={listMotion}>
            <ol
              class="grid border-t border-line"
              bind:this={list}
              aria-label={`${season.title} episodes`}
            >
              {#each episodes as episode (`${episode.season}:${episode.number}:${episode.id}`)}
                <EpisodeRow
                  {episode}
                  {show}
                  {series}
                  {admin}
                  {timezone}
                  next={!!episode.id && episode.id === next}
                  focused={!!episode.id && episode.id === focusEpisode}
                  watched={episode.watched}
                  busy={busy === episode.id}
                  toggle={() => void toggleWatched(episode)}
                  {play}
                  {changed}
                  watchedChanged={showWatched}
                />
              {:else}<li
                  class="py-6 text-sm text-muted"
                  data-motion-row="empty"
                >
                  No episode matches.
                </li>{/each}
            </ol>
          </RowMotionBody>
        </RowMotionBody>
      </div>
    {/if}
  </section>
{/if}
