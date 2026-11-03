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
    autoplay = true,
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
    /** Whether the next episode starts by itself. */
    autoplay?: boolean;
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
  // The last credits, with a preview straight after them, and where they end.
  const closing = $derived.by(() => {
    const credits = items.filter((s) => s.kind === 'Credits').at(-1);
    if (!credits || preferences.Credits === 'Ignore') return undefined;
    const preview = items.find(
      (s) => s.kind === 'Preview' && Math.abs(s.start - credits.end) <= 2,
    );
    return { credits, preview, end: preview?.end ?? credits.end };
  });
  // Credits that run to the end lead to the next episode instead of to a skip.
  // Credits followed by a scene keep their skip; the next episode waits for them.
  const finale = $derived(
    next && closing && closing.end >= duration - 2 ? closing : undefined,
  );
  const handedOver = $derived(
    finale ? [finale.credits, finale.preview].filter((s) => !!s) : [],
  );
  const credits = $derived(
    finale && {
      start: finale.credits.start,
      auto: preferences.Credits === 'Auto',
    },
  );
  const playing = (s: Segment) =>
    position >= s.start && position < s.end && preferences[s.kind] !== 'Ignore';
  const current = $derived(
    items.find((s) => !handedOver.includes(s) && playing(s)),
  );
  // Without autoplay, automatic skips still skip, up to the prompt.
  const skipped = $derived(
    current ?? (autoplay ? undefined : handedOver.find(playing)),
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
    const segment = skipped;
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
    after={finale ? undefined : closing?.end}
    {autoplay}
    play={advance}
  />{/if}
{#if current && !busy}<Button size="form" onclick={() => void skip(current!)}
    >Skip {current.kind.toLowerCase()}</Button
  >{/if}
{#if error}<p role="alert">{error}</p>{/if}
