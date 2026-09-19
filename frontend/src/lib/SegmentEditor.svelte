<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from './api';
  let { mediaId, fileId, duration, episode } = $props<{
    mediaId: string;
    fileId: string;
    duration: number;
    episode: boolean;
  }>();
  type Segment = {
    id: string;
    kind: string;
    start: number;
    end: number;
    source: string;
    confidence: number;
  };
  let items = $state<Segment[]>([]),
    generation = $state(''),
    message = $state(''),
    busy = $state(false);
  let revision = 0;
  $effect(() => {
    const ids = [mediaId, fileId];
    untrack(() => void load(ids));
  });
  async function load([media, file]: string[]) {
    const rev = ++revision;
    busy = true;
    try {
      const data = await api<{ items: Segment[]; generation: string }>(
        `/catalog/${media}/segments?file_id=${encodeURIComponent(file)}`,
      );
      if (rev === revision) {
        items = data.items;
        generation = data.generation;
      }
    } catch (e) {
      message = String(e);
    } finally {
      if (rev === revision) busy = false;
    }
  }
  async function save(reset = false) {
    busy = true;
    message = '';
    try {
      await api(`/catalog/${mediaId}/segments`, 'PUT', {
        file_id: fileId,
        generation,
        items,
        reset,
      });
      await load([mediaId, fileId]);
      message = reset
        ? 'Automatic timestamps restored'
        : 'Manual corrections saved';
    } catch (e) {
      message = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<details class="segments">
  <summary>Intro, recap, credits and preview timestamps</summary>
  <p>
    Times are seconds within this edition. Saving an empty list suppresses
    automatic timestamps. Corrections apply to this file version.
  </p>
  {#each items as item (item.id)}<div class="segment-row">
      <label
        >Type<select bind:value={item.kind}
          >{#each ['Intro', 'Recap', 'Credits', 'Preview'] as kind (kind)}<option
              >{kind}</option
            >{/each}</select
        ></label
      >
      <label
        >Start<input
          type="number"
          min="0"
          max={duration}
          step="0.1"
          bind:value={item.start}
        /></label
      >
      <label
        >End<input
          type="number"
          min="0"
          max={duration}
          step="0.1"
          bind:value={item.end}
        /></label
      >
      <span>{item.source} · {Math.round(item.confidence * 100)}%</span>
      <button
        class="secondary"
        disabled={busy}
        onclick={() => (items = items.filter((s) => s.id !== item.id))}
        >Remove</button
      >
    </div>{/each}
  <div class="actions">
    <button
      class="secondary"
      disabled={busy || items.length >= 40}
      onclick={() =>
        (items = [
          ...items,
          {
            id: crypto.randomUUID(),
            kind: 'Intro',
            start: 0,
            end: Math.min(30, duration),
            source: 'manual',
            confidence: 1,
          },
        ])}>Add segment</button
    >
    <button
      class="primary"
      disabled={busy || !generation}
      onclick={() => void save()}>Save corrections</button
    >
    <button
      class="secondary"
      disabled={busy || !generation}
      onclick={() => void save(true)}>Use automatic timestamps</button
    >
    {#if episode}<button
        class="secondary"
        disabled={busy}
        onclick={async () => {
          try {
            await api(
              `/catalog/${mediaId}/segments/analyze?file_id=${encodeURIComponent(fileId)}`,
              'POST',
            );
            message = 'Analysis queued; refresh after it finishes';
          } catch (e) {
            message = String(e);
          }
        }}>Reanalyze episode</button
      >{/if}
    <button
      class="secondary"
      disabled={busy}
      onclick={() => void load([mediaId, fileId])}>Refresh segments</button
    >
  </div>
  {#if message}<p role="status">{message}</p>{/if}
</details>

<style>
  .segments {
    margin: 1rem 0;
    padding: 1rem;
    border: 1px solid var(--line);
  }
  summary {
    cursor: pointer;
    font-weight: 600;
    margin-bottom: 1rem;
  }
  .segment-row,
  .actions {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    flex-wrap: wrap;
    margin: 0.8rem 0;
  }
  label {
    margin: 0;
    max-width: 9rem;
  }
  span {
    font-size: 12px;
    color: var(--muted);
  }
</style>
