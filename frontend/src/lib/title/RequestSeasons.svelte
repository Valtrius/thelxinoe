<script lang="ts">
  import Modal from '../ui/Modal.svelte';
  import Notice from '../ui/Notice.svelte';
  import RequestButton, {
    type ProfileRef,
  } from '../seerr/RequestButton.svelte';
  import { availability, seasonRequested, type Media } from '../seerr/types';

  let { media, busy, error, request, close } = $props<{
    media: Media;
    busy: boolean;
    error: string;
    request: (seasons: number[], profile?: ProfileRef) => void;
    close: () => void;
  }>();
  const seasons = $derived(
    (media.seasons ?? []).filter(
      (s: NonNullable<Media['seasons']>[number]) =>
        s.seasonNumber >= 0 && s.episodeCount > 0,
    ),
  );
  const open = $derived(
    seasons.filter(
      (s: (typeof seasons)[number]) => !seasonRequested(media, s.seasonNumber),
    ),
  );
  // Every regular season that can still be requested starts selected.
  let selected = $derived<number[]>(
    open
      .filter((s: (typeof seasons)[number]) => s.seasonNumber > 0)
      .map((s: (typeof seasons)[number]) => s.seasonNumber),
  );
</script>

<Modal title="Request seasons" {busy} onClose={close}>
  {#if error}<Notice variant="error" role="alert">{error}</Notice>{/if}
  {#if open.length}<label
      class="mb-1 flex cursor-pointer items-center gap-2 border-b border-line pb-3 text-xs"
      ><input
        class="accent-accent"
        type="checkbox"
        checked={selected.length === open.length}
        onchange={(event) =>
          (selected = event.currentTarget.checked
            ? open.map((s: (typeof seasons)[number]) => s.seasonNumber)
            : [])}
      /> Select all available seasons</label
    >{/if}
  <div class="max-h-85 overflow-y-auto">
    {#each seasons as season (season.id)}{@const requested = seasonRequested(
        media,
        season.seasonNumber,
      )}<label
        class="flex items-center gap-3 border-b border-line py-3 text-xs last:border-0"
        ><input
          type="checkbox"
          class="accent-accent"
          checked={selected.includes(season.seasonNumber)}
          onchange={(event) =>
            (selected = event.currentTarget.checked
              ? [...selected, season.seasonNumber]
              : selected.filter((n: number) => n !== season.seasonNumber))}
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
                (s: { seasonNumber: number; status: number }) =>
                  s.seasonNumber === season.seasonNumber,
              )?.status,
            ) || 'Requested'}</span
          >{/if}</label
      >{/each}
  </div>
  <RequestButton
    mediaKind="tv"
    id={media.id}
    {busy}
    disabled={!selected.length || busy}
    inactiveLabel="Select seasons"
    inline
    request={(profile) => request(selected, profile)}
  />
</Modal>
