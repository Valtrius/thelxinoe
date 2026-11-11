<script lang="ts">
  import { DropdownMenu } from 'bits-ui';
  import { ListMusic, LoaderCircle } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import MenuBackdrop, { ignoreBackdrop } from '../ui/MenuBackdrop.svelte';
  import type { MusicItem } from './library';
  import type { Playlist } from './playlists.svelte';
  import { addToPlaylist, addablePlaylists } from './track-actions';

  let {
    tracks,
    disabled = false,
  }: {
    /** Read when a playlist is chosen, since an artist's tracks can be many. */
    tracks: () => MusicItem[] | Promise<MusicItem[]>;
    disabled?: boolean;
  } = $props();
  let open = $state(false),
    adding = $state(false),
    playlists = $state<Playlist[] | null>(null);
  const itemClass =
    'flex h-8 w-full cursor-default items-center whitespace-nowrap px-[0.55rem] text-left text-[0.7rem] text-muted outline-none data-[highlighted]:bg-accent-soft data-[highlighted]:text-foreground compact:h-11 compact:text-[13px]';

  async function add(list: Playlist) {
    adding = true;
    try {
      await addToPlaylist(list, await tracks());
    } finally {
      adding = false;
    }
  }
</script>

<DropdownMenu.Root
  bind:open
  onOpenChange={async (value) => {
    if (!value) return;
    playlists = null;
    playlists = await addablePlaylists();
  }}
>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}<Button
        {...props}
        variant="secondary"
        size="form"
        {disabled}
        loading={adding}><ListMusic size={15} />Add to playlist</Button
      >{/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    {#if open}<MenuBackdrop close={() => (open = false)} />{/if}
    <DropdownMenu.Content
      align="start"
      sideOffset={4}
      preventScroll={false}
      onInteractOutside={ignoreBackdrop}
      class="z-50 grid max-h-[60vh] max-w-80 min-w-48 overflow-y-auto border border-line-strong bg-surface-strong p-1.5 shadow-2xl"
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
            onSelect={() => void add(playlist)}
            ><span class="truncate">{playlist.name}</span></DropdownMenu.Item
          >{/each}{/if}
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
