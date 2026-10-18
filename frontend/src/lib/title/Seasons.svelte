<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { Check } from '@lucide/svelte';
  import PlayIcon from '../ui/PlayIcon.svelte';
  import Button from '../ui/Button.svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TitleMenu from './TitleMenu.svelte';
  import { showToast } from '../providers/toasts';
  import { episodeLabel, setFlag, type Episode, type Season } from './catalog';

  let {
    seasons,
    show,
    admin,
    timezone,
    focusSeason,
    focusEpisode,
    play,
    changed,
  } = $props<{
    seasons: Season[];
    show: string;
    admin: boolean;
    timezone: string;
    focusSeason?: number | null;
    focusEpisode?: string | null;
    play: (episode: Episode, fileId?: string) => void;
    changed: () => void;
  }>();
  const ids = $props.id();
  let selected = $state<number | null>(null),
    busy = $state<string | null>(null),
    // Watched toggles show immediately; the reloaded seasons replace them.
    toggled = $state<Record<string, boolean>>({}),
    list = $state<HTMLElement>(),
    tabs = $state<HTMLElement>();
  // The linked season is chosen once; later refreshes keep the reader's own tab.
  let applied: number | null = null;
  const watched = (episode: Episode) =>
    (episode.id ? toggled[episode.id] : undefined) ?? episode.watched;
  const label = (episode: Episode) => episodeLabel(episode.title, show);
  const season = $derived(
    seasons.find((entry: Season) => entry.number === selected) ?? seasons[0],
  );

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
        selected =
          seasons.find((s: Season) => s.number > 0 && s.episodes.length)
            ?.number ??
          seasons[0]?.number ??
          null;
    });
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

  const airDate = (seconds: number) =>
    new Intl.DateTimeFormat(undefined, {
      timeZone: timezone,
      day: 'numeric',
      month: 'short',
      year: 'numeric',
    }).format(new Date(seconds * 1000));

  async function toggleWatched(episode: Episode) {
    if (!episode.id) return;
    busy = episode.id;
    const value = !watched(episode);
    try {
      await setFlag(episode.id, 'watched', value);
      toggled[episode.id] = value;
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

  // Arrow keys move between season tabs, as in any tab list.
  function navigate(event: KeyboardEvent) {
    const order = seasons.map((s: Season) => s.number);
    const index = order.indexOf(season?.number);
    const next =
      event.key === 'ArrowRight'
        ? order[(index + 1) % order.length]
        : event.key === 'ArrowLeft'
          ? order[(index - 1 + order.length) % order.length]
          : event.key === 'Home'
            ? order[0]
            : event.key === 'End'
              ? order.at(-1)
              : undefined;
    if (next === undefined) return;
    event.preventDefault();
    selected = next;
    void tick().then(() =>
      tabs?.querySelector<HTMLElement>('[aria-selected="true"]')?.focus(),
    );
  }
</script>

{#if seasons.length}
  <section class="mt-9" aria-label="Seasons">
    <div class="mb-4 flex items-end justify-between gap-3 border-b border-line">
      <div
        bind:this={tabs}
        class="flex min-w-0 gap-1 overflow-x-auto"
        role="tablist"
        aria-label="Seasons"
        tabindex="-1"
        onkeydown={navigate}
      >
        {#each seasons as entry (entry.number)}<button
            id={`${ids}-tab-${entry.number}`}
            role="tab"
            aria-selected={entry.number === season?.number}
            aria-controls={`${ids}-panel`}
            tabindex={entry.number === season?.number ? 0 : -1}
            class={[
              'shrink-0 border-b-2 px-4 pt-2 pb-3 text-xs font-semibold transition-colors hover:text-foreground',
              entry.number === season?.number
                ? 'border-accent text-accent'
                : 'border-transparent text-muted',
            ]}
            onclick={() => (selected = entry.number)}>{entry.title}</button
          >{/each}
      </div>
      {#if season?.id}<div class="mb-1.5 shrink-0">
          {#key season.id}<TitleMenu
              id={season.id}
              kind="season"
              title={season.title}
              {admin}
              variant="row"
              {changed}
            />{/key}
        </div>{/if}
    </div>
    {#if season}
      <div
        id={`${ids}-panel`}
        role="tabpanel"
        aria-labelledby={`${ids}-tab-${season.number}`}
      >
        {#if season.status}<p class="mb-3">
            <StatusBadge status={season.status} {timezone} />
          </p>{/if}
        <ol
          class="grid"
          bind:this={list}
          aria-label={`${season.title} episodes`}
        >
          {#each season.episodes as episode (`${episode.season}:${episode.number}:${episode.id}`)}
            <li
              data-episode={episode.id ?? undefined}
              class={[
                'grid grid-cols-[2rem_minmax(0,1fr)_auto] items-start gap-x-3 border-b border-line py-3.5 last:border-b-0',
                episode.id &&
                  episode.id === focusEpisode &&
                  'bg-accent-soft/50',
                !episode.id && 'opacity-70',
              ]}
            >
              {#if episode.available && episode.id}<Button
                  variant="ghost"
                  size="compact-icon"
                  aria-label={`Play episode ${episode.number}: ${label(episode)}`}
                  onclick={() => play(episode)}
                  ><PlayIcon media={episode.id} /></Button
                >{:else}<span
                  class="grid size-7.5 place-items-center text-xs text-muted tabular-nums"
                  >{episode.number}</span
                >{/if}
              <div class="min-w-0">
                <p class="text-sm font-medium">
                  <span class="mr-1 text-muted tabular-nums"
                    >{episode.number}.</span
                  >{label(episode)}
                </p>
                <p
                  class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted"
                >
                  {#if episode.air_date && episode.status?.state !== 'upcoming'}<span
                      >{airDate(episode.air_date)}</span
                    >{/if}{#if episode.runtime}<span>{episode.runtime} min</span
                    >{/if}{#if episode.status && episode.status.state !== 'available'}<StatusBadge
                      status={episode.status}
                      {timezone}
                      compact
                    />{/if}
                </p>
                {#if episode.overview}<p
                    class="mt-1.5 line-clamp-2 max-w-190 text-xs leading-5 text-muted"
                  >
                    {episode.overview}
                  </p>{/if}
                {#if episode.position && episode.duration && !watched(episode)}<ProgressBar
                    class="mt-2 h-0.75 w-40 bg-line"
                    value={(episode.position / episode.duration) * 100}
                  />{/if}
              </div>
              <div class="flex items-center gap-1">
                {#if episode.id}<Button
                    variant="ghost"
                    size="compact-icon"
                    class={watched(episode) ? 'text-accent' : ''}
                    aria-pressed={watched(episode)}
                    aria-label={`Watched: ${label(episode)}`}
                    title={watched(episode) ? 'Mark unwatched' : 'Mark watched'}
                    disabled={busy === episode.id}
                    onclick={() => void toggleWatched(episode)}
                    ><Check size={15} /></Button
                  ><TitleMenu
                    id={episode.id}
                    kind="episode"
                    title={label(episode)}
                    {admin}
                    variant="row"
                    play={(fileId) => play(episode, fileId)}
                    {changed}
                  />{/if}
              </div>
            </li>
          {/each}
        </ol>
      </div>
    {/if}
  </section>
{/if}
