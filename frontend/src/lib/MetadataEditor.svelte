<script lang="ts">
  import Notice from './ui/Notice.svelte';
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { api } from './api';
  import { untrack } from 'svelte';
  import Button from './ui/Button.svelte';
  import { inlineFormClass, rowClass } from './ui/styles';
  let {
    id,
    kind,
    title,
    heading = true,
  } = $props<{
    id: string;
    kind: string;
    title: string;
    heading?: boolean;
  }>();
  type Candidate = {
    service_id: string;
    service_generation: string;
    source: string;
    external_id: string;
    title: string;
    year?: string;
    overview?: string;
  };
  let query = $state(''),
    candidates = $state<Candidate[]>([]),
    error = $state(''),
    message = $state(''),
    busy = $state(false);
  $effect(() => {
    if (id)
      untrack(() => {
        query = title;
        candidates = [];
        message = '';
      });
  });
  async function run(fn: () => Promise<void>) {
    error = '';
    message = '';
    busy = true;
    try {
      await fn();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class={heading ? 'mt-5 border-t border-line pt-4.5' : ''}>
  {#if heading}<h3 class="text-[15px] font-[550]">Metadata</h3>{/if}
  {#if error}<Notice variant="error" role="alert">{error}</Notice
    >{/if}{#if message}<p class="text-[0.75rem] text-muted" role="status">
      {message}
    </p>{/if}
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void run(async () => {
        candidates = (
          await api<{ items: Candidate[] }>(
            `/metadata/search?kind=${kind}&q=${encodeURIComponent(query)}`,
          )
        ).items;
        if (!candidates.length) message = 'No matches found.';
      });
    }}
  >
    <FormField
      >Find a match<input
        class={formControlClass}
        bind:value={query}
        required
      /></FormField
    ><Button type="submit" variant="secondary" size="form" disabled={busy}
      >Search media service</Button
    >
  </form>
  {#each candidates as candidate (`${candidate.service_id}:${candidate.external_id}`)}<div
      class={rowClass}
    >
      <div>
        <strong>{candidate.title}</strong><small
          >{candidate.year ?? candidate.source}</small
        >
      </div>
      <Button
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() =>
          run(async () => {
            await api(`/catalog/${id}/match`, 'POST', {
              service_id: candidate.service_id,
              service_generation: candidate.service_generation,
              external_id: candidate.external_id,
            });
            message = 'Match queued.';
            candidates = [];
          })}>Use this match</Button
      >
    </div>{/each}
</div>
