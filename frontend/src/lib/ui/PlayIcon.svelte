<script lang="ts">
  import { LoaderCircle, Play } from '@lucide/svelte';
  import { launchingMedia } from '../native-playback';
  let { media, size = 15 } = $props<{
    media: string | null | undefined;
    size?: number;
  }>();
</script>

<!-- A play button's icon spins while its video starts in a desktop MPV window. -->
{#if media && launchingMedia.has(media)}<LoaderCircle
    {size}
    class="animate-spin motion-reduce:animate-none"
    aria-hidden="true"
    data-launching
  />{:else}<Play {size} />{/if}
