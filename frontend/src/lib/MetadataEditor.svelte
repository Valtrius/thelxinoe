<script lang="ts">
  import { api } from './api';
  import { untrack } from 'svelte';
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

<div class="metadata-editor">
  <h3>Metadata</h3>
  {#if error}<p class="error" role="alert">{error}</p>{/if}{#if message}<p
      role="status"
    >
      {message}
    </p>{/if}
  {#if ['movie', 'show', 'artist', 'album', 'track'].includes(kind)}<form
      class="inline-form"
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
      <label>Find a match<input bind:value={query} required /></label><button
        class="secondary"
        disabled={busy}>Search provider</button
      ><button
        type="button"
        class="secondary"
        disabled={busy}
        onclick={() =>
          run(async () => {
            await api(`/catalog/${id}/refresh`, 'POST');
            message =
              'Refresh queued. Watch Background jobs in Settings for its result.';
          })}>Refresh metadata</button
      >
    </form>
    {#each candidates as candidate (candidate.id)}<div class="row">
        <div>
          <strong>{candidate.title}</strong><small
            >{candidate.year ?? candidate.provider}</small
          >
        </div>
        <button
          class="secondary"
          disabled={busy}
          onclick={() =>
            run(async () => {
              await api(`/catalog/${id}/match`, 'POST', {
                provider: candidate.provider,
                external_id: candidate.id,
              });
              message = 'Match queued.';
              candidates = [];
            })}>Use this match</button
        >
      </div>{/each}{/if}
  <form
    class="inline-form"
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
    ><button class="secondary" disabled={busy}>Save corrections</button><button
      class="secondary"
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
        })}>Clear corrections</button
    >
  </form>
</div>

<style>
  .metadata-editor {
    border-top: 1px solid var(--line);
    padding-top: 18px;
    margin-top: 20px;
  }
  h3 {
    font-weight: 550;
    font-size: 15px;
  }
  .metadata-editor > p {
    font-size: 12px;
    color: var(--muted);
  }
</style>
