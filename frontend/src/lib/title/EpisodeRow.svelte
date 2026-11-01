<script lang="ts">
  import {
    BookmarkCheck,
    CalendarClock,
    Check,
    Film,
    ScanSearch,
    Zap,
  } from '@lucide/svelte';
  import PlayIcon from '../ui/PlayIcon.svelte';
  import Button from '../ui/Button.svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import { dateFormat } from '../intl';
  import { formatBytes } from '../providers/utils';
  import { showToast } from '../providers/toasts';
  import EpisodeStill from './EpisodeStill.svelte';
  import ReleaseSearch from './ReleaseSearch.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TitleMenu from './TitleMenu.svelte';
  import WatchedToggle from './WatchedToggle.svelte';
  import { episodeLabel, type Episode } from './catalog';
  import { episodeCode, phase, resumeText } from './episodes';
  import { searchEpisodes } from './sonarr';
  import { day } from './status';

  let {
    episode,
    show,
    series,
    admin,
    timezone,
    next = false,
    focused = false,
    watched,
    busy = false,
    toggle,
    play,
    changed,
  } = $props<{
    episode: Episode;
    /** The series name, which episode titles from files may repeat. */
    show: string;
    /** The series as its seasons load, for Sonarr's actions. */
    series: string | null;
    admin: boolean;
    timezone: string;
    /** The episode Play on the title page starts. */
    next?: boolean;
    focused?: boolean;
    watched: boolean;
    busy?: boolean;
    toggle: () => void;
    play: (episode: Episode, fileId?: string) => void;
    changed: () => void;
  }>();
  let searching = $state(false),
    releases = $state(false);
  const label = $derived(episodeLabel(episode.title, show));
  const name = $derived(`${episodeCode(episode)} · ${label}`);
  const current = $derived(phase(episode));
  const playable = $derived(episode.available && !!episode.id);
  const resume = $derived(resumeText(episode));
  const progress = $derived(
    resume && episode.position && episode.duration
      ? episode.position / episode.duration
      : null,
  );
  const managed = $derived(admin && !!series && !!episode.manager_id);
  const airDate = (seconds: number) =>
    dateFormat(undefined, {
      timeZone: timezone,
      day: 'numeric',
      month: 'short',
      year: 'numeric',
    }).format(new Date(seconds * 1000));

  async function search(monitor: boolean) {
    if (!series || !episode.manager_id) return;
    searching = true;
    try {
      await searchEpisodes(series, {
        episodes: [episode.manager_id],
        monitor,
      });
      showToast({
        key: `episode-search:${episode.manager_id}`,
        tone: 'success',
        title: name,
        message: monitor
          ? 'Sonarr monitors it and is searching for it.'
          : 'Sonarr is searching for it.',
      });
      if (monitor) changed();
    } catch (caught) {
      showToast({
        key: `episode-search:${episode.manager_id}`,
        tone: 'error',
        title: name,
        message: String(caught),
      });
    } finally {
      searching = false;
    }
  }
</script>

{#snippet picture()}
  <EpisodeStill
    stills={episode.stills}
    class={[
      watched && 'opacity-45',
      current === 'downloading' && 'opacity-60',
      ['missing', 'unmonitored', 'unavailable'].includes(current) &&
        'opacity-30 grayscale',
    ]}
  >
    {#snippet placeholder()}
      {#if current === 'upcoming' && episode.air_date}<CalendarClock
          size={15}
        /><span>{day(episode.air_date, timezone, Date.now())}</span>{:else}<Film
          size={15}
        />{/if}
    {/snippet}
  </EpisodeStill>
  {#if watched}<span
      class="absolute top-1.5 left-1.5 grid size-4 place-items-center rounded-full bg-accent text-background"
      ><Check size={10} strokeWidth={3.2} /></span
    >{/if}
  {#if episode.runtime}<span
      class="absolute right-1 bottom-1.5 bg-background/80 px-1 text-[9.5px] text-foreground tabular-nums"
      >{episode.runtime}m</span
    >{/if}
  {#if progress}<ProgressBar
      class="absolute inset-x-0 bottom-0 h-0.75 bg-white/20"
      value={progress * 100}
    />{/if}
{/snippet}

<li
  data-episode={episode.id ?? undefined}
  class={[
    'grid grid-cols-[8.5rem_minmax(0,1fr)_auto] items-start gap-x-3.5 border-b border-line px-2 py-3 last:border-b-0 compact:grid-cols-[6.5rem_minmax(0,1fr)_auto] compact:gap-x-3',
    next && 'bg-accent-soft shadow-[inset_2px_0_0_var(--accent)]',
    focused && !next && 'bg-accent-soft/50',
  ]}
>
  {#if playable}<button
      type="button"
      class="group relative aspect-video overflow-hidden border border-white/8 bg-surface-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
      aria-label={`Play episode ${episode.number}: ${label}`}
      onclick={() => play(episode)}
      >{@render picture()}<span
        class={[
          'absolute inset-0 grid place-items-center transition-opacity motion-reduce:transition-none',
          next
            ? 'opacity-100'
            : 'opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 touch:opacity-100',
        ]}
        ><span
          class="grid size-7.5 place-items-center rounded-full border border-white/40 bg-background/60 text-white"
          ><PlayIcon media={episode.id} size={13} /></span
        ></span
      ></button
    >{:else}<div
      class="relative aspect-video overflow-hidden border border-white/8 bg-surface-strong"
    >
      {@render picture()}
    </div>{/if}
  <div class="min-w-0 compact:contents">
    <div class="min-w-0 compact:contents">
      <p
        class={[
          'mb-0 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm font-medium',
          watched && 'text-foreground/80',
          current === 'upcoming' && !episode.id && 'text-muted',
        ]}
      >
        <span
          ><span class="mr-1 text-muted tabular-nums">{episode.number}.</span
          >{label}</span
        >{#if next}<span
            class="border border-line-strong px-1.5 pt-0.5 pb-px text-[8.5px] font-bold tracking-[0.12em] text-accent uppercase"
            >Next up</span
          >{/if}
      </p>
      <p
        class="mt-1 mb-0 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted empty:hidden compact:col-span-full compact:mt-2"
      >
        {#if episode.air_date && current !== 'upcoming'}<span
            >{airDate(episode.air_date)}</span
          >{/if}{#if resume}<span class="text-accent">{resume}</span
          >{/if}{#if episode.status && episode.status.state !== 'available'}<StatusBadge
            status={episode.status}
            {timezone}
            compact
          />{/if}{#if admin && episode.file && (episode.file.quality || episode.file.size)}<span
            class="inline-flex items-center gap-1.5"
            ><Film size={12} class="shrink-0" />{[
              episode.file.quality,
              episode.file.size ? formatBytes(episode.file.size) : '',
            ]
              .filter(Boolean)
              .join(' · ')}</span
          >{/if}
      </p>
    </div>
    {#if episode.overview}<p
        class="mt-1.5 mb-0 line-clamp-2 max-w-190 text-xs leading-5 text-muted compact:col-span-full compact:mt-2"
      >
        {episode.overview}
      </p>{/if}
    {#if managed && current === 'missing'}<div
        class="mt-2.5 flex flex-wrap gap-1.5 compact:col-span-full"
      >
        <Button size="sm" loading={searching} onclick={() => void search(false)}
          ><Zap size={13} />Search</Button
        ><Button variant="secondary" size="sm" onclick={() => (releases = true)}
          ><ScanSearch size={13} />Interactive search…</Button
        >
      </div>{:else if managed && current === 'unmonitored'}<div
        class="mt-2.5 compact:col-span-full"
      >
        <Button
          variant="secondary"
          size="sm"
          loading={searching}
          onclick={() => void search(true)}
          ><BookmarkCheck size={13} />Monitor and search</Button
        >
      </div>{/if}
  </div>
  <div
    class="flex items-center gap-0.5 compact:col-start-3 compact:row-start-1"
  >
    {#if episode.id}<WatchedToggle
        {watched}
        {progress}
        {label}
        {busy}
        {toggle}
      />{:else}<span class="size-7.5" aria-hidden="true"></span>{/if}
    <TitleMenu
      id={episode.id}
      kind="episode"
      title={label}
      {admin}
      variant="row"
      sonarr={series && episode.manager_id
        ? {
            series,
            season: episode.season,
            episode: episode.manager_id,
          }
        : null}
      play={(fileId) => play(episode, fileId)}
      {changed}
    />
  </div>
  {#if releases && series && episode.manager_id}<ReleaseSearch
      {series}
      target={{ episode: episode.manager_id }}
      title={name}
      close={() => (releases = false)}
    />{/if}
</li>
