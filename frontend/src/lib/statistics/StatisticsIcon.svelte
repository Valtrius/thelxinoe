<script lang="ts">
  import { Film, Tv, Music } from '@lucide/svelte';
  import PlatformIcon from '../ui/PlatformIcon.svelte';
  import type { StatisticsSource } from './types';
  let { platform, class: className = '' } = $props<{
    platform: StatisticsSource;
    class?: string;
  }>();
</script>

{#if platform === 'movies'}<Film class={className} />
{:else if platform === 'shows'}<Tv class={className} />
{:else if platform === 'music'}<Music class={className} />
{:else}<PlatformIcon {platform} class={className} />{/if}
