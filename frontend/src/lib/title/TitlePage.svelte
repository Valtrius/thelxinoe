<script lang="ts">
  import { revisions } from '../server-events.svelte';
  import { onDestroy, untrack } from 'svelte';
  import {
    ArrowLeft,
    Bookmark,
    BookmarkCheck,
    ExternalLink,
    Heart,
    Plus,
    Star,
  } from '@lucide/svelte';
  import { api, type User } from '../api';
  import { display } from '../display.svelte';
  import { inBackground } from '../page-readiness';
  import type { MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import PlayIcon from '../ui/PlayIcon.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaRow from '../seerr/MediaRow.svelte';
  import { compactViewport } from '../responsive';
  import { forgetSeerrStatus, seerrStatus } from '../seerr/status';
  import RequestButton, {
    type ProfileRef,
  } from '../seerr/RequestButton.svelte';
  import {
    artwork,
    availability,
    backdropSet,
    posterSet,
    kind as seerrKind,
    seasonRequested,
    type Media,
    type MediaKind,
    type Results,
  } from '../seerr/types';
  import Seasons from './Seasons.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import StatusChip from '../ui/StatusChip.svelte';
  import TitleMenu from './TitleMenu.svelte';
  import RequestSeasons from './RequestSeasons.svelte';
  import { hasStatus, isTitleRef, type TitleStatus } from './status';
  import {
    asset,
    loadFlags,
    loadItem,
    loadSeasons,
    lookup,
    metadataNumber,
    seriesOf,
    setFlag,
    episodeLabel,
    type CatalogItem,
    type Episode,
    type Flags,
    type ManagerTitle,
    type Season,
    type TitleKind,
  } from './catalog';

  let { user, kind, mediaId, tmdb, back, backLabel, play, openTitle } = $props<{
    user: User;
    kind: TitleKind;
    /** A library id (any level of a series) or a `tmdb:`/`tvdb:` reference. */
    mediaId?: string;
    /** TMDB id from Discover. */
    tmdb?: number;
    back: () => void;
    backLabel: string;
    play: (choice: MediaChoice) => void;
    openTitle: (target: { kind: MediaKind; id: number }) => void;
  }>();
  const admin = $derived(user.role === 'admin');
  let item = $state<CatalogItem | null>(null),
    manager = $state<ManagerTitle | null>(null),
    lookedUpStatus = $state<TitleStatus | null>(null),
    seerr = $state<Media | null>(null),
    related = $state<Media[]>([]),
    seasons = $state<Season[]>([]),
    flags = $state<Flags | null>(null),
    focusSeason = $state<number | null>(null),
    focusEpisode = $state<string | null>(null),
    loading = $state(true),
    error = $state(''),
    requestBusy = $state(false),
    requestError = $state(''),
    requestMessage = $state(''),
    choosingSeasons = $state(false),
    flagBusy = $state(false),
    unreachable = $state(false);
  let generation = 0;
  // Titles outside the library are found again by this reference on every refresh.
  const reference = $derived(
    mediaId && isTitleRef(mediaId) ? mediaId : tmdb ? `tmdb:${tmdb}` : null,
  );
  const status = $derived<TitleStatus | null>(
    item?.status ?? lookedUpStatus ?? null,
  );
  const tmdbId = $derived(
    tmdb ??
      metadataNumber(item, 'tmdb_id') ??
      manager?.tmdb_id ??
      (mediaId?.startsWith('tmdb:') ? Number(mediaId.slice(5)) : undefined),
  );
  const name = $derived(
    item?.title ?? seerr?.title ?? seerr?.name ?? manager?.title ?? '',
  );
  const year = $derived(
    item?.year ??
      manager?.year ??
      (Number((seerr?.releaseDate || seerr?.firstAirDate || '').slice(0, 4)) ||
        null),
  );
  const overview = $derived(
    (item?.overview as string | null | undefined) ||
      seerr?.overview ||
      manager?.overview ||
      '',
  );
  const poster = $derived(
    artwork(seerr?.posterPath) ||
      asset(item?.artwork_url) ||
      asset(manager?.poster_url),
  );
  const backdrop = $derived(
    artwork(seerr?.backdropPath, 'w1280') ||
      asset(item?.backdrop_url) ||
      asset(manager?.backdrop_url),
  );
  const genres = $derived<string[]>(
    seerr?.genres?.map((g: { name: string }) => g.name) ??
      (Array.isArray(item?.metadata?.genres)
        ? (item!.metadata.genres as string[])
        : (manager?.genres ?? [])),
  );
  const ended = $derived(
    kind === 'show' &&
      (item?.ended ??
        manager?.ended ??
        ['Ended', 'Canceled'].includes(seerr?.status ?? '')),
  );
  const rating = $derived(
    seerr?.voteAverage || metadataNumber(item, 'vote_average'),
  );
  const runtime = $derived(
    seerr?.runtime || metadataNumber(item, 'runtime') || manager?.runtime,
  );
  const present = $derived(item?.files?.filter((f) => f.present) ?? []);
  const trailer = $derived(
    seerr?.relatedVideos?.find(
      (v: { site: string; type: string; key: string }) =>
        v.site === 'YouTube' &&
        v.type === 'Trailer' &&
        /^[\w-]{11}$/.test(v.key),
    ),
  );
  // The next episode to play: one in progress, else the first unwatched.
  const nextEpisode = $derived.by(() => {
    const episodes = seasons
      .filter((s) => s.number > 0)
      .flatMap((s) => s.episodes)
      .filter((e) => e.id && e.available);
    return (
      episodes.find((e) => !e.watched && e.position) ??
      episodes.find((e) => !e.watched) ??
      episodes[0]
    );
  });
  // Seasons load from the library show, else from Sonarr by the title's reference.
  const series = $derived(
    item?.id ?? (reference && manager ? reference : null),
  );
  // Seerr's own state, for titles Radarr and Sonarr don't track yet.
  const requestState = $derived(
    !status && seerr ? availability(seerr.mediaInfo?.status) : '',
  );
  const movieRequestable = $derived(
    kind === 'movie' &&
      !!seerr &&
      !item?.available &&
      ![2, 3, 4, 5, 6].includes(seerr.mediaInfo?.status ?? 0),
  );
  const seasonsRequestable = $derived(
    kind === 'show' &&
      !!seerr &&
      (seerr.seasons ?? []).some(
        (s: { seasonNumber: number; episodeCount: number }) =>
          s.seasonNumber > 0 &&
          s.episodeCount > 0 &&
          !seasonRequested(seerr!, s.seasonNumber),
      ),
  );

  async function loadSeerr(id: number, request: number) {
    try {
      unreachable = false;
      const state = await seerrStatus();
      if (!state.configured || !state.ready) return;
      const path = `/seerr/${kind === 'show' ? 'tv' : 'movie'}/${id}`;
      const media = await api<Media>(path);
      if (request !== generation) return;
      seerr = media;
      // The row at the foot of the page arrives after it shows.
      void inBackground(() => api<Results>(`${path}/recommendations`))
        .then((result) => {
          if (request === generation)
            related = result.results.map((entry) => ({
              ...entry,
              mediaType: kind === 'show' ? 'tv' : 'movie',
            }));
        })
        .catch(() => {});
    } catch {
      forgetSeerrStatus();
      // Seerr adds artwork, cast and requests; the library page works without it.
      if (request === generation) unreachable = true;
    }
  }

  // Rereads what the page shows. A title outside the library is looked up again,
  // since it may have started downloading or been imported since. The first load
  // has just read the item, so it passes `reread = false`.
  async function refresh(request = generation, reread = true) {
    let id = item?.id;
    let fresh = !reread;
    if (!id && reference && reread) {
      const result = await lookup(kind, reference);
      if (request !== generation) return;
      manager = result.title;
      lookedUpStatus = result.status;
      if (result.media_id) {
        const found = await loadItem(result.media_id);
        if (request !== generation) return;
        item = found;
        id = found.id;
        fresh = true;
      }
    }
    // Sonarr can list a series' seasons before any of its files are in the library.
    const source = id ?? (reference && manager ? reference : null);
    const [nextItem, nextFlags, nextSeasons] = await Promise.all([
      id && !fresh ? loadItem(id) : null,
      id ? loadFlags(id) : null,
      kind === 'show' && source
        ? loadSeasons(source)
            .then((r) => r.seasons)
            .catch((caught) => (id ? Promise.reject(caught) : []))
        : [],
    ]);
    if (request !== generation) return;
    if (nextItem) item = nextItem;
    flags = nextFlags;
    seasons = nextSeasons;
  }

  async function load() {
    const request = ++generation;
    loading = true;
    error = '';
    item = manager = seerr = flags = lookedUpStatus = null;
    related = seasons = [];
    focusSeason = focusEpisode = null;
    requestMessage = requestError = '';
    requestBusy = choosingSeasons = flagBusy = unreachable = false;
    try {
      if (mediaId && !isTitleRef(mediaId)) {
        let found = await loadItem(mediaId);
        let season: number | null = null,
          episode: string | null = null;
        if (['season', 'episode'].includes(found.kind)) {
          const { show, chain } = await seriesOf(found);
          season =
            chain.find((entry) => entry.kind === 'season')?.sort_number ?? null;
          episode = chain.find((entry) => entry.kind === 'episode')?.id ?? null;
          found = show;
        }
        if (request !== generation) return;
        item = found;
        focusSeason = season;
        focusEpisode = episode;
      } else if (reference) {
        // Without the library match, Discover titles still show Seerr's details.
        const result = await lookup(kind, reference).catch(() => {
          unreachable = true;
          return null;
        });
        if (request !== generation) return;
        manager = result?.title ?? null;
        lookedUpStatus = result?.status ?? null;
        if (result?.media_id) {
          const found = await loadItem(result.media_id);
          if (request !== generation) return;
          item = found;
        }
      }
      if (request !== generation) return;
      await Promise.all([
        refresh(request, false),
        tmdbId ? loadSeerr(tmdbId, request) : null,
      ]);
      if (request === generation && !item && !manager && !seerr)
        error = unreachable
          ? 'This title could not be loaded. Check the connection and try again.'
          : 'This title is not in your library, Radarr, Sonarr or Seerr anymore.';
    } catch (caught) {
      if (request === generation) error = String(caught);
    } finally {
      if (request === generation) loading = false;
    }
  }

  $effect(() => {
    void mediaId;
    void tmdb;
    void kind;
    untrack(() => void load());
  });
  $effect(() => {
    if (revisions.catalog > 0)
      untrack(() => {
        if (!loading) void refresh().catch(() => {});
      });
  });
  onDestroy(() => generation++);

  async function toggle(key: 'watch_later' | 'favorite') {
    if (!item || !flags || flagBusy) return;
    flagBusy = true;
    const request = generation;
    try {
      const next = await setFlag(item.id, key, !flags[key]);
      if (request === generation) flags = next;
    } catch (caught) {
      if (request === generation) error = String(caught);
    } finally {
      if (request === generation) flagBusy = false;
    }
  }

  function playMovie(fileId?: string) {
    if (item) play({ id: item.id, title: item.title, kind: 'movie', fileId });
  }
  function playEpisode(episode: Episode, fileId?: string) {
    if (episode.id)
      play({
        id: episode.id,
        title: `${name} · ${episodeLabel(episode.title, name)}`,
        kind: 'episode',
        fileId,
      });
  }

  async function requestMedia(selected: number[], profile?: ProfileRef) {
    if (!seerr || requestBusy) return;
    requestBusy = true;
    requestError = '';
    requestMessage = '';
    const request = generation;
    try {
      const result = await api<{ status: number }>('/seerr/requests', 'POST', {
        media_type: seerrKind(seerr),
        media_id: seerr.id,
        seasons: kind === 'show' ? selected : [],
        ...(profile ? { profile } : {}),
      });
      if (request !== generation) return;
      requestMessage =
        result.status === 1
          ? 'Request sent for approval.'
          : 'Request submitted.';
      choosingSeasons = false;
      await Promise.all([
        loadSeerr(seerr.id, request),
        refresh(request).catch(() => {}),
      ]);
    } catch (caught) {
      if (request === generation) requestError = String(caught);
    } finally {
      if (request === generation) requestBusy = false;
    }
  }
</script>

<div class="mb-5">
  <Button variant="ghost" size="sm" onclick={back}
    ><ArrowLeft size={16} /> {backLabel}</Button
  >
</div>
{#if loading}<ContentSkeleton label="Loading details" variant="detail" />
{:else if error && !item && !manager && !seerr}<Notice
    variant="error"
    role="alert"
    >{error}<Button variant="ghost" size="sm" onclick={() => void load()}
      >Reload details</Button
    ></Notice
  >
{:else}
  <article class="min-w-0" aria-label={name}>
    <div
      class="relative isolate min-h-95 overflow-hidden border border-line bg-surface compact:min-h-80"
    >
      {#if backdrop}<img
          src={backdrop}
          srcset={backdropSet(seerr?.backdropPath)}
          sizes="100vw"
          alt=""
          class="absolute inset-0 -z-2 size-full object-cover object-top opacity-45"
        />{/if}
      <div
        class="absolute inset-0 -z-1 bg-linear-to-t from-background via-background/65 to-background/5"
      ></div>
      <div
        class="flex min-h-95 items-end gap-7 p-8 compact:min-h-80 compact:gap-4 compact:p-5"
      >
        <!-- Hidden images still download: compact layouts leave the poster out. -->
        {#if poster && !compactViewport.current}<img
            src={poster}
            srcset={posterSet(seerr?.posterPath)}
            sizes="10rem"
            alt=""
            class="w-40 shrink-0 border border-white/15 shadow-xl"
          />{/if}
        <div class="max-w-190 min-w-0 py-1">
          <p
            class="mb-3 text-[11px] font-semibold tracking-widest text-accent uppercase"
          >
            {kind === 'movie' ? 'Movie' : 'Series'}{year ? ` · ${year}` : ''}
            {#if ended}<StatusChip dot={false} class="ml-2 align-[1px]"
                >Ended</StatusChip
              >{/if}
          </p>
          <h1
            class="text-4xl leading-tight font-semibold tracking-tight compact:text-3xl"
          >
            {name}
          </h1>
          {#if seerr?.tagline}<p class="mt-3 text-sm text-muted">
              {seerr.tagline}
            </p>{/if}
          <div
            class="mt-5 flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-muted"
          >
            {#if rating}<span class="flex items-center gap-1.5 text-foreground"
                ><Star size={14} class="text-amber-300" />{rating.toFixed(
                  1,
                )}</span
              >{/if}{#if runtime && kind === 'movie'}<span>{runtime} min</span
              >{/if}{#if kind === 'show' && seasons.some((s) => s.number > 0)}<span
                >{seasons.filter((s) => s.number > 0).length}
                {seasons.filter((s) => s.number > 0).length === 1
                  ? 'season'
                  : 'seasons'}</span
              >{/if}{#each genres.slice(0, 4) as genre (genre)}<span
                >{genre}</span
              >{/each}
          </div>
          {#if hasStatus(status)}<p class="mt-5">
              <StatusBadge {status} timezone={display.timezone} />
            </p>{:else if requestState && kind === 'show'}<p
              class="mt-5 text-xs font-semibold text-accent"
            >
              {requestState}
            </p>{/if}
          <div class="mt-5 flex flex-wrap items-center gap-2">
            {#if kind === 'movie' && item?.available}<Button
                size="form"
                onclick={() => playMovie()}
                ><PlayIcon media={item.id} />Play</Button
              >{:else if kind === 'show' && nextEpisode}<Button
                size="form"
                onclick={() => playEpisode(nextEpisode)}
                ><PlayIcon media={nextEpisode.id} />{nextEpisode.position &&
                !nextEpisode.watched
                  ? 'Resume'
                  : 'Play'}
                S{nextEpisode.season}E{nextEpisode.number}</Button
              >{/if}
            {#if kind === 'movie' && seerr && !item?.available && (movieRequestable || requestState)}<div
                class="w-56 [&>div]:mt-0"
              >
                <RequestButton
                  mediaKind="movie"
                  id={seerr.id}
                  busy={requestBusy}
                  disabled={!movieRequestable || requestBusy}
                  inactiveLabel={requestState || 'Requested'}
                  request={(profile) => void requestMedia([], profile)}
                />
              </div>{:else if seasonsRequestable}<Button
                size="form"
                variant={nextEpisode ? 'secondary' : 'default'}
                onclick={() => {
                  requestError = '';
                  choosingSeasons = true;
                }}
                ><Plus size={15} />{item
                  ? 'Request more seasons'
                  : 'Request'}</Button
              >{/if}
            {#if item && flags}<Button
                variant="secondary"
                size="form"
                aria-pressed={flags.watch_later}
                disabled={flagBusy}
                onclick={() => void toggle('watch_later')}
                >{#if flags.watch_later}<BookmarkCheck
                    size={15}
                    class="text-accent"
                  />{:else}<Bookmark size={15} />{/if}Watch Later</Button
              ><Button
                variant="secondary"
                size="icon"
                aria-pressed={flags.favorite}
                aria-label="Favorite"
                title={flags.favorite
                  ? 'Remove from Favorites'
                  : 'Add to Favorites'}
                disabled={flagBusy}
                onclick={() => void toggle('favorite')}
                ><Heart
                  size={15}
                  class={flags.favorite ? 'fill-accent text-accent' : ''}
                /></Button
              ><TitleMenu
                id={item.id}
                {kind}
                title={name}
                {admin}
                watchLater={false}
                favorite={false}
                tmdbLink={false}
                play={kind === 'movie' ? playMovie : undefined}
                changed={() => void refresh().catch(() => {})}
              />{:else if admin && kind === 'movie' && reference && manager}<TitleMenu
                id={null}
                {kind}
                title={name}
                {admin}
                radarr={reference}
                changed={() => void load()}
              />{/if}
          </div>
          {#if requestMessage}<p role="status" class="mt-3 text-xs text-accent">
              {requestMessage}
            </p>{/if}
          {#if requestError && !choosingSeasons}<p
              role="alert"
              class="mt-3 text-xs text-danger"
            >
              {requestError}
            </p>{/if}
        </div>
      </div>
    </div>
    {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
    <div class="mt-7 min-w-0">
      <h2 class="mb-3 text-sm font-semibold">Overview</h2>
      <p class="max-w-190 text-sm leading-7 text-muted">
        {overview || 'No overview available.'}
      </p>
      <div class="mt-5 flex flex-wrap gap-3">
        {#if trailer}<a
            class="inline-flex items-center gap-2 border border-line px-3 py-2 text-xs hover:border-accent"
            href={`https://www.youtube.com/watch?v=${trailer.key}`}
            data-playback-title={`${name} · ${trailer.name}`}
            target="_blank"
            rel="noreferrer"
            ><PlayIcon media={`youtube:${trailer.key}`} size={13} /> Watch trailer</a
          >{/if}{#if tmdbId}<a
            class="inline-flex items-center gap-2 px-3 py-2 text-xs text-muted hover:text-foreground"
            href={`https://www.themoviedb.org/${kind === 'show' ? 'tv' : 'movie'}/${tmdbId}`}
            target="_blank"
            rel="noreferrer">TMDB <ExternalLink size={13} /></a
          >{/if}
      </div>
      {#if kind === 'movie' && present.length > 1}<p
          class="mt-4 text-xs text-muted"
        >
          {present.length} editions: {present
            .map((file) => file.edition || 'Original')
            .join(', ')}. Choose one from ⋯ › Play edition.
        </p>{/if}
    </div>
    {#if kind === 'show'}<Seasons
        {seasons}
        show={name}
        {series}
        next={nextEpisode?.id}
        {admin}
        timezone={display.timezone}
        {focusSeason}
        {focusEpisode}
        play={playEpisode}
        changed={() => void refresh().catch(() => {})}
      />{/if}
    {#if seerr?.credits?.cast?.length}<section class="mt-9" aria-label="Cast">
        <h2 class="mb-4 text-sm font-semibold">Cast</h2>
        <div class="flex gap-4 overflow-x-auto pb-3">
          {#each seerr.credits.cast.slice(0, 12) as person (person.id)}<div
              class="w-22 shrink-0"
            >
              {#if artwork(person.profilePath)}<img
                  src={artwork(person.profilePath, 'w185')}
                  alt=""
                  loading="lazy"
                  class="aspect-2/3 w-full object-cover"
                />{:else}<div class="aspect-2/3 bg-surface-strong"></div>{/if}
              <p class="mt-2 text-[11px] font-medium">{person.name}</p>
              <p class="mt-1 text-[10px] text-muted">{person.character}</p>
            </div>{/each}
        </div>
      </section>{/if}
    {#if related.length}<div class="mt-9">
        <MediaRow
          label="Recommendations"
          items={related}
          open={(media: Media) =>
            openTitle({ kind: seerrKind(media), id: media.id })}
        />
      </div>{/if}
  </article>
  {#if choosingSeasons && seerr}<RequestSeasons
      media={seerr}
      busy={requestBusy}
      error={requestError}
      request={(selected: number[], profile?: ProfileRef) =>
        void requestMedia(selected, profile)}
      close={() => (choosingSeasons = false)}
    />{/if}
{/if}
