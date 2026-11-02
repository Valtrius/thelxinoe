<script lang="ts" module>
  export type NextEpisode = {
    id: string;
    title: string;
    show: string;
    season: number;
    number: number;
  };
</script>

<script lang="ts">
  import { untrack } from 'svelte';
  import { Play, X } from '@lucide/svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import { episodeLabel } from './title/catalog';
  import { episodeCode } from './title/episodes';
  import type { MediaChoice } from './playback';
  let {
    mediaId,
    fileId,
    next = $bindable(null),
    position,
    duration,
    paused,
    busy = false,
    credits,
    play,
  } = $props<{
    mediaId: string;
    fileId: string;
    /** The episode after this one, once loaded. */
    next?: NextEpisode | null;
    position: number;
    duration: number;
    paused: boolean;
    busy?: boolean;
    /** Where the credits start, and whether they skip by themselves. */
    credits?: { start: number; auto: boolean };
    play: (choice: MediaChoice) => void;
  }>();
  let revision = 0;
  $effect(() => {
    const key = [mediaId, fileId];
    untrack(() => void load(key));
  });
  async function load([media, file]: string[]) {
    const rev = ++revision;
    next = null;
    try {
      const data = await api<{ next: NextEpisode | null }>(
        `/catalog/${media}/next?file_id=${encodeURIComponent(file)}`,
      );
      if (rev === revision) next = data.next;
    } catch {
      /* Without a next episode, playback stops at the end as before. */
    }
  }

  // Credits lead to the next episode after a short countdown. Without them it is
  // offered when Jellyfin offers it, 30 to 40 seconds before the end, and plays
  // when this one ends.
  const countdown = 10;
  const lead = $derived(duration >= 3000 ? 40 : duration >= 2400 ? 35 : 30);
  const offerAt = $derived(credits ? credits.start : duration - lead);
  let dismissed = $state(false),
    timer = $state(countdown),
    started = false;
  const shown = $derived(
    !!next && duration > 0 && position >= offerAt && !dismissed,
  );
  const left = $derived(
    Math.ceil(Math.min(credits ? timer : Infinity, duration - position)),
  );
  const code = $derived(next ? episodeCode(next) : '');
  const label = $derived(next ? episodeLabel(next.title, next.show) : '');
  const name = $derived(label === code ? code : `${code} · ${label}`);
  function start() {
    if (!next || started) return;
    started = true;
    play({ id: next.id, kind: 'episode', title: `${next.show} · ${label}` });
  }
  // Going back before the offer brings it back.
  $effect(() => {
    if (position < offerAt) {
      dismissed = false;
      timer = countdown;
    }
  });
  $effect(() => {
    if (!shown || !credits || paused || busy) return;
    const tick = setInterval(() => (timer = Math.max(0, timer - 1)), 1000);
    return () => clearInterval(tick);
  });
  $effect(() => {
    if (shown && !busy && (left <= 0 || (credits?.auto && !paused)))
      untrack(start);
  });
</script>

{#if shown && left > 0 && !credits?.auto}<div
    class="mb-2 flex max-w-[calc(100cqw-2rem)] justify-end gap-1"
    role="group"
    aria-label="Up next"
  >
    <Button
      size="form"
      class="min-w-0"
      aria-label={`Play ${name}`}
      title={`Play ${name}`}
      onclick={start}
      ><Play size={14} fill="currentColor" class="shrink-0" /><span
        class="truncate">{name}</span
      ><span class="tabular-nums opacity-70">{left}</span></Button
    >
    <Button
      size="form"
      aria-label="Cancel autoplay"
      title="Cancel autoplay"
      onclick={() => (dismissed = true)}><X size={14} /></Button
    >
  </div>{/if}
