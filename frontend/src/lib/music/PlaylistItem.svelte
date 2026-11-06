<script lang="ts">
  import {
    Check,
    Disc3,
    Info,
    ListEnd,
    ListMusic,
    ListStart,
    MicVocal,
    Play,
    X,
  } from '@lucide/svelte';
  import { time } from '../playback';
  import { touchDevice } from '../responsive';
  import ListSidebarCard from '../ui/ListSidebarCard.svelte';
  import ProgressBar from '../ui/ProgressBar.svelte';
  import MusicArt from './MusicArt.svelte';
  import TrackMenu from './TrackMenu.svelte';
  import { musicPlayer } from './player.svelte';
  import type { PlaylistTrack } from './playlist-order';
  import type { Playlist } from './playlists.svelte';
  import { addToPlaylist, addablePlaylists, queueTrack } from './track-actions';

  let {
    track,
    canRemove,
    sortable = false,
    dragging = false,
    suppressClick = false,
    play,
    remove,
    details: showDetails,
    openAlbum,
    openArtist,
    onReorderStart,
  }: {
    track: PlaylistTrack;
    canRemove: boolean;
    sortable?: boolean;
    dragging?: boolean;
    suppressClick?: boolean;
    play: () => void;
    remove: () => void;
    details: () => void;
    openAlbum?: () => void;
    openArtist?: () => void;
    onReorderStart?: (event: PointerEvent) => void;
  } = $props();

  const isPlaying = $derived(musicPlayer.state?.id === track.id);
  const length = $derived(
    (isPlaying && musicPlayer.state?.duration) || track.duration || 0,
  );
  const position = $derived(isPlaying ? (musicPlayer.state?.position ?? 0) : 0);
  let choices = $state<Playlist[] | null>(null);
  let choosing = $state(false);

  function chooseList() {
    choosing = !choosing;
    if (!choosing) return;
    choices = null;
    void addablePlaylists(track.id).then((lists) => (choices = lists));
  }
</script>

<ListSidebarCard
  title={track.title}
  canPlay={track.available}
  {isPlaying}
  {sortable}
  {dragging}
  {suppressClick}
  closeActionsLabel="Close track actions"
  class={`h-[53px] ${track.available || dragging ? '' : 'opacity-50'}`}
  textClass="justify-center"
  titleClass="mb-0 truncate"
  onPlay={play}
  {onReorderStart}
>
  {#snippet thumbnail()}
    <div class="relative size-[52px] shrink-0 overflow-hidden">
      <MusicArt url={track.artwork_url} class="size-full" sizes="52px" lazy />
      {#if isPlaying && length > 0}
        <ProgressBar
          value={(position / length) * 100}
          class="absolute inset-x-0 bottom-0 bg-white/12"
          barClass="shadow-accent-glow"
        />
      {/if}
    </div>
  {/snippet}
  {#snippet titleEnd()}
    <span class="shrink-0 font-mono text-[0.6rem] text-muted"
      >{isPlaying && length > 0
        ? `-${time(length - position)}`
        : track.duration
          ? time(track.duration)
          : ''}</span
    >
  {/snippet}
  {#snippet details()}
    <p
      class="mt-0.5 mb-0 truncate pr-15 text-[0.62rem] leading-3.5 tracking-[0.08em] text-muted uppercase touch:pr-0"
    >
      {track.artist ?? ''}
    </p>
  {/snippet}
  {#snippet actions({ actionClass, close })}
    {#if touchDevice.current}
      {#if track.available}
        <button
          type="button"
          class={actionClass}
          onclick={() => {
            close();
            play();
          }}><Play class="size-3.5" /><span>Play</span></button
        >
        {#if musicPlayer.controls}
          <button
            type="button"
            class={actionClass}
            onclick={() => {
              close();
              queueTrack(track, true);
            }}><ListStart class="size-3.5" /><span>Play next</span></button
          >
          <button
            type="button"
            class={actionClass}
            onclick={() => {
              close();
              queueTrack(track, false);
            }}><ListEnd class="size-3.5" /><span>Add to queue</span></button
          >
        {/if}
      {/if}
      <button
        type="button"
        class={actionClass}
        aria-expanded={choosing}
        onclick={chooseList}
        ><ListMusic class="size-3.5" /><span>Add to playlist</span></button
      >
      {#if choosing}
        {#if choices && !choices.length}
          <p class="mb-0 py-2 pl-11 text-sm text-muted">
            No playlists to add to.
          </p>
        {/if}
        {#each choices ?? [] as list (list.id)}
          <button
            type="button"
            class={`${actionClass} pl-11 disabled:opacity-45`}
            disabled={list.contains}
            onclick={() => {
              close();
              void addToPlaylist(list, [track]);
            }}
            >{#if list.contains}<Check class="size-3.5 shrink-0" />{/if}<span
              class="truncate"
              >{list.contains ? `In ${list.name}` : list.name}</span
            ></button
          >
        {/each}
      {/if}
      {#if openAlbum}
        <button
          type="button"
          class={actionClass}
          onclick={() => {
            close();
            openAlbum();
          }}><Disc3 class="size-3.5" /><span>Go to album</span></button
        >
      {/if}
      {#if openArtist}
        <button
          type="button"
          class={actionClass}
          onclick={() => {
            close();
            openArtist();
          }}><MicVocal class="size-3.5" /><span>Go to artist</span></button
        >
      {/if}
      <button
        type="button"
        class={actionClass}
        onclick={() => {
          close();
          showDetails();
        }}><Info class="size-3.5" /><span>Details</span></button
      >
    {:else}
      <TrackMenu
        {track}
        {play}
        details={showDetails}
        {openAlbum}
        {openArtist}
      />
    {/if}
    {#if canRemove}
      <button
        type="button"
        class={`${actionClass} hover:text-danger`}
        title="Remove from playlist"
        aria-label={touchDevice.current
          ? undefined
          : `Remove ${track.title} from the playlist`}
        onclick={() => {
          close();
          remove();
        }}
        ><X class="size-3.5" />{#if touchDevice.current}<span
            >Remove from playlist</span
          >{/if}</button
      >
    {/if}
  {/snippet}
</ListSidebarCard>
