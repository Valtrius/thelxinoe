<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import {
    Check,
    Disc3,
    Ellipsis,
    Info,
    ListEnd,
    ListMusic,
    ListStart,
    LoaderCircle,
    MicVocal,
    Play,
  } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import MenuSubContent from '../ui/MenuSubContent.svelte';
  import { musicPlayer } from './player.svelte';
  import type { MusicItem } from './library';
  import type { Playlist } from './playlists.svelte';
  import { addToPlaylist, addablePlaylists, queueTrack } from './track-actions';

  let { track, play, details, openAlbum, openArtist } = $props<{
    track: MusicItem;
    play: () => void;
    details: () => void;
    /** Left out on the album's own page. */
    openAlbum?: () => void;
    /** Left out on the artist's own page. */
    openArtist?: () => void;
  }>();
  let open = $state(false),
    playlists = $state<Playlist[] | null>(null);
  const itemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[disabled]:opacity-45 data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] [&_svg]:shrink-0 compact:h-11 compact:text-[13px]';
  // Requested when the menu opens, so rows don't each read the playlists.
  async function loadPlaylists() {
    playlists = null;
    playlists = await addablePlaylists(track.id);
  }
</script>

<DropdownMenu.Root
  bind:open
  onOpenChange={(value) => {
    if (value) void loadPlaylists();
  }}
>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}<Button
        {...props}
        variant="ghost"
        size="compact-icon"
        class="track-menu"
        aria-label={`Actions for ${track.title}`}
        title="More actions"><Ellipsis size={16} /></Button
      >{/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    {#if open}<MenuBackdrop close={() => (open = false)} />{/if}
    <DropdownMenu.Content
      align="end"
      sideOffset={4}
      preventScroll={false}
      onInteractOutside={ignoreBackdrop}
      class="z-50 grid min-w-48 border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
    >
      {#if track.available}
        <DropdownMenu.Item class={itemClass} onSelect={play}
          ><Play />Play</DropdownMenu.Item
        >
        {#if musicPlayer.controls}
          <DropdownMenu.Item
            class={itemClass}
            onSelect={() => queueTrack(track, true)}
            ><ListStart />Play next</DropdownMenu.Item
          >
          <DropdownMenu.Item
            class={itemClass}
            onSelect={() => queueTrack(track, false)}
            ><ListEnd />Add to queue</DropdownMenu.Item
          >
        {/if}
      {/if}
      <DropdownMenu.Sub>
        <DropdownMenu.SubTrigger class={itemClass}
          ><ListMusic />Add to playlist</DropdownMenu.SubTrigger
        >
        <MenuSubContent
          class="max-w-80 min-w-48 compact:max-w-[calc(100vw-1rem)]"
        >
          {#if !playlists}<p
              class="mb-0 flex items-center gap-2 px-[0.55rem] py-1.5 text-[0.7rem] text-muted"
            >
              <LoaderCircle
                size={13}
                class="animate-spin motion-reduce:animate-none"
              />Loading playlists
            </p>{:else if !playlists.length}<p
              class="mb-0 px-[0.55rem] py-1.5 text-[0.7rem] text-muted"
            >
              No playlists to add to.
            </p>{:else}{#each playlists as playlist (playlist.id)}<DropdownMenu.Item
                class={itemClass}
                disabled={playlist.contains}
                onSelect={() => void addToPlaylist(playlist, [track])}
                >{#if playlist.contains}<Check />{/if}<span class="truncate"
                  >{playlist.contains
                    ? `In ${playlist.name}`
                    : playlist.name}</span
                ></DropdownMenu.Item
              >{/each}{/if}
        </MenuSubContent>
      </DropdownMenu.Sub>
      {#if openAlbum || (openArtist && track.artist_id)}
        <DropdownMenu.Separator class="my-1 h-px bg-line" />
        {#if openAlbum}<DropdownMenu.Item class={itemClass} onSelect={openAlbum}
            ><Disc3 />Go to album</DropdownMenu.Item
          >{/if}
        {#if openArtist && track.artist_id}<DropdownMenu.Item
            class={itemClass}
            onSelect={openArtist}><MicVocal />Go to artist</DropdownMenu.Item
          >{/if}
      {/if}
      <DropdownMenu.Separator class="my-1 h-px bg-line" />
      <DropdownMenu.Item class={itemClass} onSelect={details}
        ><Info />Details…</DropdownMenu.Item
      >
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
