<script lang="ts">
  import { api } from './api';
  import { untrack } from 'svelte';
  import Button from './ui/Button.svelte';
  import { errorClass, inlineFormClass, rowClass } from './ui/styles';
  let {
    id,
    kind,
    title,
    overrides = {},
    changed,
  } = $props<{
    id: string;
    kind: string;
    title: string;
    overrides?: { title?: string; overview?: string; year?: number };
    changed: () => void;
  }>();
  type Candidate = {
    provider: string;
    id: string;
    title: string;
    year?: string;
    overview?: string;
  };
  let query = $state(''),
    manualTitle = $state(''),
    overview = $state(''),
    year = $state<number | undefined>(),
    candidates = $state<Candidate[]>([]),
    error = $state(''),
    message = $state(''),
    busy = $state(false);
  $effect(() => {
    if (id)
      untrack(() => {
        query = title;
        manualTitle = overrides.title ?? '';
        overview = overrides.overview ?? '';
        year = overrides.year;
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

<div class="mt-5 border-t border-line pt-4.5">
  <h3 class="text-[15px] font-[550]">Metadata</h3>
  {#if error}<p class={errorClass} role="alert">{error}</p>{/if}{#if message}<p
      class="text-[0.75rem] text-muted"
      role="status"
    >
      {message}
    </p>{/if}
  {#if ['movie', 'show', 'artist', 'album', 'track'].includes(kind)}<form
      class={inlineFormClass}
      onsubmit={(e) => {
        e.preventDefault();
        void run(async () => {
          candidates = (
            await api<{ items: Candidate[] }>(
              `/metadata/search?kind=${kind}&q=${encodeURIComponent(query)}`,
            )
          ).items;
        });
      }}
    >
      <label>Find a match<input bind:value={query} required /></label><Button
        type="submit"
        variant="secondary"
        size="form"
        disabled={busy}>Search provider</Button
      ><Button
        type="button"
        variant="secondary"
        size="form"
        disabled={busy}
        onclick={() =>
          run(async () => {
            await api(`/catalog/${id}/refresh`, 'POST');
            message =
              'Refresh queued. Watch Background jobs in Settings for its result.';
          })}>Refresh metadata</Button
      >
    </form>
    {#each candidates as candidate (candidate.id)}<div class={rowClass}>
        <div>
          <strong>{candidate.title}</strong><small
            >{candidate.year ?? candidate.provider}</small
          >
        </div>
        <Button
          variant="secondary"
          size="form"
          disabled={busy}
          onclick={() =>
            run(async () => {
              await api(`/catalog/${id}/match`, 'POST', {
                provider: candidate.provider,
                external_id: candidate.id,
              });
              message = 'Match queued.';
              candidates = [];
            })}>Use this match</Button
        >
      </div>{/each}{/if}
  <form
    class={inlineFormClass}
    onsubmit={(e) => {
      e.preventDefault();
      void run(async () => {
        await api(`/catalog/${id}/overrides`, 'PUT', {
          ...(manualTitle ? { title: manualTitle } : {}),
          ...(overview ? { overview } : {}),
          ...(year ? { year } : {}),
        });
        message =
          'Corrections saved. They take precedence over provider metadata.';
        changed();
      });
    }}
  >
    <label
      >Manual title<input
        bind:value={manualTitle}
        placeholder="Leave empty to use provider title"
      /></label
    ><label
      >Manual overview<input
        bind:value={overview}
        placeholder="Leave empty to use provider overview"
      /></label
    ><label
      >Manual year<input
        type="number"
        min="1"
        max="9999"
        bind:value={year}
        placeholder="Use provider year"
      /></label
    ><Button type="submit" variant="secondary" size="form" disabled={busy}
      >Save corrections</Button
    ><Button
      variant="secondary"
      size="form"
      disabled={busy}
      type="button"
      onclick={() =>
        run(async () => {
          await api(`/catalog/${id}/overrides`, 'PUT', {});
          manualTitle = '';
          overview = '';
          year = undefined;
          changed();
          message = 'Provider metadata restored.';
        })}>Clear corrections</Button
    >
  </form>
</div>
