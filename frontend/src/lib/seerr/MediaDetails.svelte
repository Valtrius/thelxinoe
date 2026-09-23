<script lang="ts">
  import {
    ArrowLeft,
    Check,
    Plus,
    LoaderCircle,
    Star,
    Play,
    ExternalLink,
  } from '@lucide/svelte';
  import { onDestroy } from 'svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import Notice from '../ui/Notice.svelte';
  import MediaRow from './MediaRow.svelte';
  import {
    artwork,
    availability,
    title,
    year,
    seasonRequested,
    type Media,
    type MediaKind,
    type Results,
  } from './types';
  let { mediaKind, id, back, open } = $props<{
    mediaKind: MediaKind;
    id: number;
    back: () => void;
    open: (item: Media) => void;
  }>();
  let media = $state<Media | null>(null),
    related = $state<Media[]>([]),
    selected = $state<number[]>([]),
    busy = $state(false),
    loading = $state(true),
    error = $state(''),
    message = $state('');
  let generation = 0;
  const status = $derived(availability(media?.mediaInfo?.status));
  const seasons = $derived(
    (media?.seasons ?? []).filter(
      (s) => s.seasonNumber >= 0 && s.episodeCount > 0,
    ),
  );
  const availableSeasons = $derived(
    seasons.filter((s) => media && !seasonRequested(media, s.seasonNumber)),
  );
  const canRequest = $derived(
    !!media &&
      (mediaKind === 'tv'
        ? selected.length > 0
        : ![2, 3, 4, 5, 6].includes(media.mediaInfo?.status ?? 0)),
  );
  const trailer = $derived(
    media?.relatedVideos?.find(
      (v) =>
        v.site === 'YouTube' &&
        v.type === 'Trailer' &&
        /^[\w-]{11}$/.test(v.key),
    ),
  );
  async function load() {
    const request = ++generation;
    const path = `/seerr/${mediaKind}/${id}`;
    loading = true;
    error = '';
    message = '';
    media = null;
    related = [];
    selected = [];
    try {
      const result = await api<Media>(path);
      if (request !== generation) return;
      media = result;
      selected = (result.seasons ?? [])
        .filter(
          (s) =>
            s.seasonNumber > 0 &&
            s.episodeCount > 0 &&
            !seasonRequested(result, s.seasonNumber),
        )
        .map((s) => s.seasonNumber);
      void api<Results>(`${path}/recommendations`)
        .then((result) => {
          if (request === generation)
            related = result.results.map((item) => ({
              ...item,
              mediaType: mediaKind,
            }));
        })
        .catch(() => {});
    } catch (caught) {
      if (request === generation) error = String(caught);
    } finally {
      if (request === generation) loading = false;
    }
  }
  $effect(() => {
    void id;
    void mediaKind;
    void load();
  });
  onDestroy(() => generation++);
  async function requestMedia() {
    if (!canRequest || busy) return;
    busy = true;
    error = '';
    message = '';
    const version = generation,
      requestKind = mediaKind,
      requestId = id;
    try {
      const result = await api<{ status: number }>('/seerr/requests', 'POST', {
        media_type: requestKind,
        media_id: requestId,
        seasons: requestKind === 'tv' ? selected : [],
      });
      if (version !== generation) return;
      message =
        result.status === 1
          ? 'Request sent for approval.'
          : 'Request submitted.';
      if (media) {
        const nextStatus = result.status === 1 ? 2 : 3;
        media.mediaInfo = {
          ...media.mediaInfo,
          status:
            mediaKind === 'movie'
              ? nextStatus
              : (media.mediaInfo?.status ?? nextStatus),
          seasons: [
            ...(media.mediaInfo?.seasons ?? []),
            ...selected.map((seasonNumber) => ({
              seasonNumber,
              status: nextStatus,
            })),
          ],
        };
      }
      selected = [];
      const updated = await api<Media>(`/seerr/${requestKind}/${requestId}`);
      if (version === generation) media = updated;
    } catch (caught) {
      if (version === generation) error = String(caught);
    } finally {
      if (version === generation) busy = false;
    }
  }
</script>

<div class="mb-5">
  <Button variant="ghost" size="sm" onclick={back}
    ><ArrowLeft size={16} /> Back to discover</Button
  >
</div>
{#if loading}<div
    class="grid min-h-100 place-items-center text-muted"
    role="status"
  >
    <LoaderCircle class="animate-spin" size={28} /><span class="sr-only"
      >Loading details</span
    >
  </div>
{:else if media}
  <article class="min-w-0">
    <div
      class="relative isolate min-h-95 overflow-hidden border border-line bg-surface compact:min-h-80"
    >
      {#if artwork(media.backdropPath)}<img
          src={artwork(media.backdropPath, 'w1280')}
          alt=""
          class="absolute inset-0 -z-2 size-full object-cover object-top opacity-45"
        />{/if}
      <div
        class="absolute inset-0 -z-1 bg-linear-to-t from-background via-background/65 to-background/5"
      ></div>
      <div
        class="flex min-h-95 items-end gap-7 p-8 compact:min-h-80 compact:gap-4 compact:p-5"
      >
        {#if artwork(media.posterPath)}<img
            src={artwork(media.posterPath)}
            alt=""
            class="w-40 shrink-0 border border-white/15 shadow-xl compact:hidden"
          />{/if}
        <div class="max-w-190 py-1">
          <p
            class="mb-3 text-[11px] font-semibold tracking-widest text-accent uppercase"
          >
            {mediaKind === 'movie' ? 'Movie' : 'Series'}{year(media)
              ? ` · ${year(media)}`
              : ''}
          </p>
          <h1
            class="text-4xl leading-tight font-semibold tracking-tight compact:text-3xl"
          >
            {title(media)}
          </h1>
          {#if media.tagline}<p class="mt-3 text-sm text-muted">
              {media.tagline}
            </p>{/if}
          <div
            class="mt-5 flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-muted"
          >
            {#if (media.voteAverage ?? 0) > 0}<span
                class="flex items-center gap-1.5 text-foreground"
                ><Star
                  size={14}
                  class="text-amber-300"
                />{media.voteAverage!.toFixed(1)}</span
              >{/if}{#if media.runtime}<span>{media.runtime} min</span
              >{/if}{#if media.numberOfSeasons}<span
                >{media.numberOfSeasons} season{media.numberOfSeasons === 1
                  ? ''
                  : 's'}</span
              >{/if}{#each media.genres ?? [] as genre (genre.id)}<span
                >{genre.name}</span
              >{/each}
          </div>
          {#if status}<p
              class={[
                'mt-5 flex items-center gap-2 text-xs font-semibold',
                media.mediaInfo?.status === 5
                  ? 'text-emerald-400'
                  : 'text-accent',
              ]}
            >
              <Check size={15} />{status}
            </p>{/if}
        </div>
      </div>
    </div>
    <div
      class="mt-7 grid grid-cols-[minmax(0,1fr)_20rem] items-start gap-8 narrow:grid-cols-1 compact:gap-6"
    >
      <div class="min-w-0">
        <h2 class="mb-3 text-sm font-semibold">Overview</h2>
        <p class="max-w-190 text-sm leading-7 text-muted">
          {media.overview || 'No overview available.'}
        </p>
        <div class="mt-5 flex flex-wrap gap-3">
          {#if trailer}<a
              class="inline-flex items-center gap-2 border border-line px-3 py-2 text-xs hover:border-accent"
              href={`https://www.youtube.com/watch?v=${trailer.key}`}
              data-playback-title={`${title(media)} · ${trailer.name}`}
              target="_blank"
              rel="noreferrer"><Play size={13} /> Watch trailer</a
            >{/if}<a
            class="inline-flex items-center gap-2 px-3 py-2 text-xs text-muted hover:text-foreground"
            href={`https://www.themoviedb.org/${mediaKind}/${id}`}
            target="_blank"
            rel="noreferrer">TMDB <ExternalLink size={13} /></a
          >
        </div>
        {#if media.credits?.cast?.length}<section
            class="mt-8"
            aria-label="Cast"
          >
            <h2 class="mb-4 text-sm font-semibold">Cast</h2>
            <div class="flex gap-4 overflow-x-auto pb-3">
              {#each media.credits.cast.slice(0, 12) as person (person.id)}<div
                  class="w-22 shrink-0"
                >
                  {#if artwork(person.profilePath)}<img
                      src={artwork(person.profilePath, 'w185')}
                      alt=""
                      loading="lazy"
                      class="aspect-2/3 w-full object-cover"
                    />{:else}<div
                      class="aspect-2/3 bg-surface-strong"
                    ></div>{/if}
                  <p class="mt-2 text-[11px] font-medium">{person.name}</p>
                  <p class="mt-1 text-[10px] text-muted">{person.character}</p>
                </div>{/each}
            </div>
          </section>{/if}
      </div>
      <section
        class="border border-line bg-surface p-5 narrow:-order-1"
        aria-label="Request media"
      >
        <h2 class="mb-4 text-sm font-semibold">
          {mediaKind === 'tv' ? 'Seasons' : 'Request movie'}
        </h2>
        {#if mediaKind === 'tv'}
          {#if availableSeasons.length}<label
              class="mb-3 flex cursor-pointer items-center gap-2 border-b border-line pb-3 text-xs"
              ><input
                class="accent-accent"
                type="checkbox"
                checked={selected.length === availableSeasons.length}
                onchange={(event) =>
                  (selected = event.currentTarget.checked
                    ? availableSeasons.map((s) => s.seasonNumber)
                    : [])}
              /> Select all available seasons</label
            >{/if}
          <div class="max-h-85 overflow-y-auto">
            {#each seasons as season (season.id)}{@const requested =
                seasonRequested(media, season.seasonNumber)}<label
                class="flex items-center gap-3 border-b border-line py-3 text-xs last:border-0"
                ><input
                  type="checkbox"
                  class="accent-accent"
                  bind:group={selected}
                  value={season.seasonNumber}
                  disabled={requested || busy}
                /><span class="min-w-0 flex-1"
                  ><strong class="font-medium"
                    >{season.name || `Season ${season.seasonNumber}`}</strong
                  ><span class="mt-1 block text-[10px] text-muted"
                    >{season.episodeCount} episodes</span
                  ></span
                >{#if requested}<span class="text-[10px] text-accent"
                    >{availability(
                      media.mediaInfo?.seasons?.find(
                        (s) => s.seasonNumber === season.seasonNumber,
                      )?.status,
                    ) || 'Requested'}</span
                  >{/if}</label
              >{/each}
          </div>
          {#if !seasons.length}<p class="mb-4 text-xs text-muted">
              No seasons are available to request yet.
            </p>{/if}
        {:else}<p class="mb-4 text-xs leading-5 text-muted">
            {status || 'Request this movie with the server’s quality settings.'}
          </p>{/if}
        <Button
          class="mt-4 w-full justify-center"
          size="form"
          disabled={!canRequest || busy}
          onclick={() => void requestMedia()}
          >{#if busy}<LoaderCircle size={15} class="animate-spin" /> Sending…{:else if canRequest}<Plus
              size={15}
            />{mediaKind === 'tv'
              ? `Request ${selected.length} season${selected.length === 1 ? '' : 's'}`
              : 'Request movie'}{:else}<Check size={15} />{status ||
              'Select seasons'}{/if}</Button
        >
        {#if message}<p role="status" class="mt-3 text-xs text-accent">
            {message}
          </p>{/if}
      </section>
    </div>
    {#if related.length}<div class="mt-9">
        <MediaRow label="Recommendations" items={related} {open} />
      </div>{/if}
  </article>
{/if}
{#if error}<Notice variant="error" role="alert"
    >{error}<Button variant="ghost" size="sm" onclick={() => void load()}
      >Reload details</Button
    ></Notice
  >{/if}
