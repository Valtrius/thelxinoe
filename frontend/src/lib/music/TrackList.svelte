<script lang="ts">
  import { Clock, Music, Play } from '@lucide/svelte';
  import { time } from '../playback';
  import { touchDevice } from '../responsive';
  import TrackMenu from './TrackMenu.svelte';
  import PlayingBars from './PlayingBars.svelte';
  import { musicPlayer } from './player.svelte';
  import { artwork, discNumber, trackNumber, type MusicItem } from './library';

  let {
    tracks,
    album = false,
    play,
    details,
    openAlbum,
    openArtist,
  } = $props<{
    tracks: MusicItem[];
    /** An album's tracklist: numbered, by disc. Other lists show each track's cover and album. */
    album?: boolean;
    play: (track: MusicItem) => void;
    details: (track: MusicItem) => void;
    openAlbum?: (track: MusicItem) => void;
    openArtist?: (track: MusicItem) => void;
  }>();
  const discs = $derived(album && new Set(tracks.map(discNumber)).size > 1);
</script>

<div data-sidebar-panel class="min-w-0 border border-line bg-surface">
  {#if album}<div
      class="flex h-7.5 items-center gap-3 border-b border-line pr-2.5 pl-2.5 text-[10px] font-[550] tracking-[0.08em] text-muted uppercase"
      aria-hidden="true"
    >
      <span class="w-6 text-right">#</span><span class="flex-1">Title</span
      ><Clock data-sidebar-resize="xy-pos" size={13} class="mr-1" />
    </div>{/if}
  <ol class="m-0 list-none p-0" aria-label={album ? 'Tracks' : 'Tracks list'}>
    {#each tracks as track, index (track.id)}
      {@const current = musicPlayer.state?.id === track.id}
      {#if discs && (index === 0 || discNumber(tracks[index - 1]) !== discNumber(track))}<li
          class="border-t border-line px-2.5 pt-4 pb-1.5 text-[10px] font-semibold tracking-[0.12em] text-muted uppercase first:border-t-0"
          aria-hidden="true"
        >
          Disc {discNumber(track)}
        </li>{/if}
      <li
        class={[
          'group/track flex h-10 items-center gap-1 border-t border-line pr-2.5 text-[12.5px] first:border-t-0 hover:bg-surface-soft compact:h-12',
          current && 'text-accent',
          !track.available && 'opacity-50',
        ]}
        data-track={track.id}
        aria-current={current ? 'true' : undefined}
      >
        <button
          type="button"
          class="flex h-full min-w-0 flex-1 items-center gap-3 bg-transparent pl-2.5 text-left text-inherit disabled:cursor-default disabled:opacity-100"
          disabled={!track.available}
          aria-label={`Play ${track.title}`}
          onclick={() => play(track)}
        >
          <span
            class="flex w-6 shrink-0 justify-end text-muted tabular-nums group-hover/track:text-accent"
          >
            {#if current}<PlayingBars
                paused={musicPlayer.state?.paused ?? true}
              />{:else if track.available}<span
                class={[
                  album ? 'group-hover/track:hidden' : 'hidden',
                  touchDevice.current && 'block!',
                ]}>{album ? (trackNumber(track) ?? index + 1) : ''}</span
              ><Play
                size={13}
                fill="currentColor"
                class={[
                  'hidden',
                  !touchDevice.current && 'group-hover/track:block',
                ]}
              />{:else if album}{trackNumber(track) ?? index + 1}{/if}
          </span>
          {#if !album}{#if track.artwork_url}<img
                src={artwork(track.artwork_url)}
                alt=""
                loading="lazy"
                class="size-7 shrink-0 border border-white/10 object-cover"
              />{:else}<span
                class="grid size-7 shrink-0 place-items-center bg-surface-strong text-muted"
                ><Music size={13} /></span
              >{/if}{/if}
          <span class="min-w-0 truncate"
            >{track.title}{#if !album && (track.artist || track.album)}<span
                class="ml-2 text-[11px] text-muted"
                >{[track.artist, track.album].filter(Boolean).join(' · ')}</span
              >{/if}{#if !track.available}<span
                class="ml-2 text-[11px] text-muted">Unavailable</span
              >{/if}</span
          >
        </button>
        <span
          class={[
            'shrink-0 transition-opacity',
            !touchDevice.current &&
              'opacity-0 group-hover/track:opacity-100 focus-within:opacity-100 has-[[data-state=open]]:opacity-100',
          ]}
        >
          <TrackMenu
            {track}
            play={() => play(track)}
            details={() => details(track)}
            openAlbum={openAlbum && track.parent_id
              ? () => openAlbum(track)
              : undefined}
            openArtist={openArtist ? () => openArtist(track) : undefined}
          />
        </span>
        <span
          class={[
            'w-10 shrink-0 text-right tabular-nums',
            current ? 'text-accent' : 'text-muted',
          ]}>{track.duration ? time(track.duration) : '—'}</span
        >
      </li>
    {/each}
  </ol>
</div>
