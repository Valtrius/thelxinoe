<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import {
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
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import { showToast } from '../providers/toasts';
  import { musicPlayer } from './player.svelte';
  import { trackChoice, type MusicItem } from './library';

  let { track, userId, play, details, openAlbum, openArtist } = $props<{
    track: MusicItem;
    userId: string;
    play: () => void;
    details: () => void;
    /** Left out on the album's own page. */
    openAlbum?: () => void;
    /** Left out on the artist's own page. */
    openArtist?: () => void;
  }>();
  let open = $state(false),
    playlists = $state<{ id: string; name: string }[] | null>(null);
  const itemClass =
    'flex h-8 w-full cursor-default items-center gap-[0.55rem] whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[disabled]:opacity-45 data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground [&_svg]:size-[0.85rem] [&_svg]:shrink-0 compact:h-11 compact:text-[13px]';
  const contentClass =
    'z-50 grid min-w-48 border border-line-strong bg-surface-strong p-1.5 shadow-2xl';
  // Requested when the menu opens, so rows don't each read the playlists.
  async function loadPlaylists() {
    if (playlists) return;
    try {
      const list = await api<{
        items: { id: string; name: string; owner_id: string }[];
      }>('/playlists');
      playlists = list.items.filter((p) => p.owner_id === userId);
    } catch {
      playlists = [];
    }
  }
  async function addTo(playlist: { id: string; name: string }) {
    try {
      const value = await api<{
        name: string;
        description: string;
        revision: number;
        items: { id: string }[];
      }>(`/playlists/${playlist.id}`);
      await api(`/playlists/${playlist.id}`, 'PUT', {
        ...value,
        items: [...value.items.map((i) => i.id), track.id],
      });
      showToast({
        key: `playlist:${playlist.id}`,
        tone: 'success',
        title: playlist.name,
        message: `Added ${track.title}.`,
      });
    } catch (e) {
      showToast({
        key: `playlist:${playlist.id}`,
        tone: 'error',
        title: playlist.name,
        message: String(e),
      });
    }
  }
  function queue(next: boolean) {
    musicPlayer.controls?.insert([trackChoice(track)], next);
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
      class={contentClass}
    >
      {#if track.available}
        <DropdownMenu.Item class={itemClass} onSelect={play}
          ><Play />Play</DropdownMenu.Item
        >
        {#if musicPlayer.controls}
          <DropdownMenu.Item class={itemClass} onSelect={() => queue(true)}
            ><ListStart />Play next</DropdownMenu.Item
          >
          <DropdownMenu.Item class={itemClass} onSelect={() => queue(false)}
            ><ListEnd />Add to queue</DropdownMenu.Item
          >
        {/if}
      {/if}
      <DropdownMenu.Sub>
        <DropdownMenu.SubTrigger class={itemClass}
          ><ListMusic />Add to playlist</DropdownMenu.SubTrigger
        >
        <DropdownMenu.SubContent class={contentClass}>
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
              Create a playlist in Playlists first.
            </p>{:else}{#each playlists as playlist (playlist.id)}<DropdownMenu.Item
                class={itemClass}
                onSelect={() => void addTo(playlist)}
                >{playlist.name}</DropdownMenu.Item
              >{/each}{/if}
        </DropdownMenu.SubContent>
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
