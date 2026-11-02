<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
  import UpNext, { type NextEpisode } from './UpNext.svelte';
  import type { MediaChoice } from './playback';
  let {
    mediaId,
    fileId,
    generation,
    position,
    duration = 0,
    paused,
    busy = false,
    seek,
    advance,
  } = $props<{
    mediaId: string;
    fileId: string;
    generation: string;
    position: number;
    duration?: number;
    paused: boolean;
    busy?: boolean;
    seek: (at: number) => Promise<unknown>;
    /** Plays the next episode; its credits then lead there instead of a skip. */
    advance?: (next: MediaChoice) => void;
  }>();
  type Segment = { id: string; kind: string; start: number; end: number };
  let items = $state<Segment[]>([]),
    preferences = $state<Record<string, string>>({}),
    handled = $state<string[]>([]),
    error = $state(''),
    next = $state<NextEpisode | null>(null);
  let revision = 0;
  $effect(() => {
    const key = [mediaId, fileId, generation];
    untrack(() => void load(key));
  });
  async function load([media, file, version]: string[]) {
    const rev = ++revision;
    items = [];
    handled = [];
    error = '';
    try {
      const data = await api<{
        generation: string;
        items: Segment[];
        preferences: Record<string, string>;
      }>(`/catalog/${media}/segments?file_id=${encodeURIComponent(file)}`);
      if (rev === revision && data.generation === version) {
        items = data.items;
        preferences = data.preferences;
      }
    } catch {
      /* Segments never prevent playback. */
    }
  }
  // With a next episode, the credits lead there instead of to a skip.
  const handedOver = $derived(
    next && preferences.Credits !== 'Ignore'
      ? items.filter((s) => s.kind === 'Credits').at(-1)
      : undefined,
  );
  const credits = $derived(
    handedOver && {
      start: handedOver.start,
      auto: preferences.Credits === 'Auto',
    },
  );
  const current = $derived(
    items.find(
      (s) =>
        s !== handedOver &&
        position >= s.start &&
        position < s.end &&
        preferences[s.kind] !== 'Ignore',
    ),
  );
  async function skip(segment: Segment) {
    handled = [...handled, segment.id];
    try {
      await seek(segment.end);
    } catch (e) {
      error = String(e);
    }
  }
  $effect(() => {
    const segment = current;
    if (
      segment &&
      preferences[segment.kind] === 'Auto' &&
      !paused &&
      !busy &&
      !handled.includes(segment.id)
    )
      untrack(() => void skip(segment));
  });
</script>

{#if advance}<UpNext
    {mediaId}
    {fileId}
    bind:next
    {position}
    {duration}
    {paused}
    {busy}
    {credits}
    play={advance}
  />{/if}
{#if current && !busy}<Button size="form" onclick={() => void skip(current!)}
    >Skip {current.kind.toLowerCase()}</Button
  >{/if}
{#if error}<p role="alert">{error}</p>{/if}
