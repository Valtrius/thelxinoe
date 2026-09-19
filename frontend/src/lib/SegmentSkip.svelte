<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  let {
    mediaId,
    fileId,
    generation,
    position,
    paused,
    busy = false,
    seek,
  } = $props<{
    mediaId: string;
    fileId: string;
    generation: string;
    position: number;
    paused: boolean;
    busy?: boolean;
    seek: (at: number) => Promise<unknown>;
  }>();
  type Segment = { id: string; kind: string; start: number; end: number };
  let items = $state<Segment[]>([]),
    preferences = $state<Record<string, string>>({}),
    handled = $state<string[]>([]),
    error = $state('');
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
  const current = $derived(
    items.find(
      (s) =>
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

{#if current && !busy}<button
    class="primary"
    onclick={() => void skip(current!)}
    >Skip {current.kind.toLowerCase()}</button
  >{/if}
{#if error}<p role="alert">{error}</p>{/if}
