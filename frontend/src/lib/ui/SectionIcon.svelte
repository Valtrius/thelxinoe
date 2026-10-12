<script lang="ts">
  import {
    House,
    Compass,
    Film,
    Tv,
    Music,
    ListMusic,
    ChartNoAxesCombined,
    Settings,
  } from '@lucide/svelte';
  import type { Section } from '../navigation';
  import PlatformIcon from './PlatformIcon.svelte';

  let {
    section,
    class: className = 'size-5 shrink-0',
  }: {
    section: Section;
    class?: string;
  } = $props();

  const icons = {
    Home: House,
    Discover: Compass,
    Movies: Film,
    Shows: Tv,
    Music,
    Playlists: ListMusic,
    YouTube: 'youtube',
    Twitch: 'twitch',
    Kick: 'kick',
    Statistics: ChartNoAxesCombined,
    Settings,
  } satisfies Record<Section, typeof House | 'youtube' | 'twitch' | 'kick'>;
  const Icon = $derived(icons[section]);
</script>

{#if typeof Icon === 'string'}
  <PlatformIcon platform={Icon} class={className} />
{:else}
  <Icon class={className} aria-hidden="true" />
{/if}
