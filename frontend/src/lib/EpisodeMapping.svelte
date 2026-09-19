<script lang="ts">
  import { api } from './api';
  import { untrack } from 'svelte';
  let { id, changed } = $props<{ id: string; changed: () => void }>();
  type Episode = {
    id: string;
    season: number;
    episode: number;
    metadata: { name?: string };
  };
  let episodes = $state<Episode[]>([]),
    chosen = $state<string[]>([]),
    error = $state(''),
    message = $state(''),
    busy = $state(false);
  let request = 0;
  async function load() {
    const generation = ++request;
    try {
      const result = await api<{
        items: Episode[];
        mappings: { id: string; state: string }[];
      }>(`/catalog/${id}/provider-episodes`);
      if (generation !== request) return;
      episodes = result.items;
      chosen = result.mappings.map((m) => m.id);
      message = result.mappings.some((m) => m.state === 'complex')
        ? 'Complex mapping: multiple logical or provider episodes share this identity.'
        : chosen.length
          ? 'Confirmed mapping.'
          : 'Unresolved: no provider episode has been selected.';
    } catch (e) {
      if (generation === request) error = String(e);
    }
  }
  $effect(() => {
    if (id)
      untrack(() => {
        error = '';
        chosen = [];
        void load();
      });
  });
  async function save() {
    busy = true;
    error = '';
    try {
      await api(`/catalog/${id}/episode-mapping`, 'PUT', {
        episode_ids: chosen,
      });
      await load();
      changed();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="mapping">
  <h3>Provider episode mapping</h3>
  <p class="muted">
    Select the episodes represented by this local item. Provider numbering can
    differ from your files.
  </p>
  {#if error}<p role="alert" class="error">{error}</p>{/if}
  <p role="status">{message}</p>
  {#if episodes.length}
    <div class="choices">
      {#each episodes as episode (episode.id)}
        <label
          ><input type="checkbox" value={episode.id} bind:group={chosen} />
          S{episode.season}E{episode.episode} · {episode.metadata.name ??
            episode.id}</label
        >
      {/each}
    </div>
    <button class="secondary" disabled={busy} onclick={save}
      >Save episode mapping</button
    >
  {:else}<p class="muted">
      Match the parent show and refresh its metadata to load provider episodes.
    </p>{/if}
</section>

<style>
  .mapping {
    margin-top: 20px;
    border-top: 1px solid var(--line);
    padding-top: 18px;
  }
  h3 {
    font-size: 15px;
    font-weight: 550;
  }
  p,
  label {
    font-size: 12px;
  }
  .choices {
    max-height: 240px;
    overflow: auto;
    margin: 12px 0;
  }
  label {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 0;
  }
  input {
    width: auto;
  }
</style>
