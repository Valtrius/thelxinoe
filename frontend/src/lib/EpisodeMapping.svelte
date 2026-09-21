<script lang="ts">
  import { api } from './api';
  import { untrack } from 'svelte';
  import Button from './ui/Button.svelte';
  import { errorClass } from './ui/styles';
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
      }>(`/catalog/${id}/manager-episodes`);
      if (generation !== request) return;
      episodes = result.items;
      chosen = result.mappings.map((m) => m.id);
      message = result.mappings.some((m) => m.state === 'complex')
        ? 'Complex mapping: multiple logical or Sonarr episodes share this identity.'
        : chosen.length
          ? 'Confirmed mapping.'
          : 'Unresolved: no Sonarr episode has been selected.';
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

<section class="mt-5 border-t border-line pt-4.5">
  <h3 class="text-[15px] font-[550]">Sonarr episode mapping</h3>
  <p class="text-[0.75rem] text-muted">
    Select the episodes represented by this local item. Sonarr numbering can
    differ from your files.
  </p>
  {#if error}<p role="alert" class={errorClass}>{error}</p>{/if}
  <p class="text-[0.75rem]" role="status">{message}</p>
  {#if episodes.length}
    <div class="my-3 max-h-60 overflow-auto">
      {#each episodes as episode (episode.id)}
        <label
          class="flex-row items-center gap-2.5 py-1.75 text-xs leading-normal"
          ><input
            class="w-auto"
            type="checkbox"
            value={episode.id}
            bind:group={chosen}
          />
          S{episode.season}E{episode.episode} · {episode.metadata.name ??
            episode.id}</label
        >
      {/each}
    </div>
    <Button variant="secondary" size="form" disabled={busy} onclick={save}
      >Save episode mapping</Button
    >
  {:else}<p class="text-[0.75rem] text-muted">
      Match the parent show and refresh its metadata to load Sonarr episodes.
    </p>{/if}
</section>
