<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { untrack } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';
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

<details
  class="my-4 border border-line p-4 [&_label]:m-0 [&_label]:max-w-36 [&_span]:text-xs [&_span]:text-muted"
>
  <summary class="mb-4 font-semibold">
    Intro, recap, credits and preview timestamps
  </summary>
  <p>
    Times are seconds within this edition. Saving an empty list suppresses
    automatic timestamps. Corrections apply to this file version.
  </p>
  {#each items as item (item.id)}<div
      class="my-[0.8rem] flex flex-wrap items-center gap-[0.8rem]"
    >
      <FormField
        >Type<select class={formControlClass} bind:value={item.kind}
          >{#each ['Intro', 'Recap', 'Credits', 'Preview'] as kind (kind)}<option
              >{kind}</option
            >{/each}</select
        ></FormField
      >
      <FormField
        >Start<input
          class={formControlClass}
          type="number"
          min="0"
          max={duration}
          step="0.1"
          bind:value={item.start}
        /></FormField
      >
      <FormField
        >End<input
          class={formControlClass}
          type="number"
          min="0"
          max={duration}
          step="0.1"
          bind:value={item.end}
        /></FormField
      >
      <span>{item.source} · {Math.round(item.confidence * 100)}%</span>
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() => (items = items.filter((s) => s.id !== item.id))}
        >Remove</Button
      >
    </div>{/each}
  <div class="my-[0.8rem] flex flex-wrap items-center gap-[0.8rem]">
    <Button
      variant="secondary"
      size="form"
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
        ])}>Add segment</Button
    >
    <Button
      size="form"
      disabled={busy || !generation}
      onclick={() => void save()}>Save corrections</Button
    >
    <Button
      variant="secondary"
      size="form"
      disabled={busy || !generation}
      onclick={() => void save(true)}>Use automatic timestamps</Button
    >
    {#if episode}<Button
        variant="secondary"
        size="form"
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
        }}>Reanalyze episode</Button
      >{/if}
    <Button
      variant="secondary"
      size="form"
      disabled={busy}
      onclick={() => void load([mediaId, fileId])}>Refresh segments</Button
    >
  </div>
  {#if message}<p role="status">{message}</p>{/if}
</details>
