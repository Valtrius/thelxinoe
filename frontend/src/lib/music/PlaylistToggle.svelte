<script lang="ts">
  import { PanelRightClose, PanelRightOpen } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import { playlists } from './playlists.svelte';

  const open = $derived(playlists.open);
</script>

<Button
  size="icon"
  variant={open ? 'secondary' : 'ghost'}
  aria-label={open ? 'Close playlists' : 'Open playlists'}
  title={open ? 'Close playlists' : 'Open playlists'}
  aria-pressed={open}
  onclick={() => playlists.setOpen(!open)}
  >{#if open}<PanelRightClose class="size-4" />{:else}<PanelRightOpen
      class="size-4"
    />{/if}</Button
>
