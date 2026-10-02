<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import { attention, acknowledgeAttention } from '../attention';
  import AttentionDot from '../ui/AttentionDot.svelte';
  import Button from '../ui/Button.svelte';
  let { navigate } = $props<{ navigate: (section: string) => void }>();
  let items = $state<
    { id: string; title: string; kind: string; state: string }[]
  >([]);
  let error = $state('');
  onMount(() => {
    let active = true;
    void api<{ items: typeof items }>('/acquisition/requests')
      .then((result) => {
        if (active) items = result.items;
      })
      .catch(() => {
        if (active) error = 'Could not load request history.';
      });
    return () => {
      active = false;
    };
  });
  function seen(id: string) {
    for (const item of $attention.filter(
      (item) => item.id === 'request:' + id && item.target === 'requests',
    ))
      void acknowledgeAttention(item);
  }
</script>

{#if error}<p role="alert" class="mt-4 text-sm text-danger">{error}</p>{/if}
{#each items as item (item.id)}
  <article
    class="mt-3 flex items-center gap-4 border border-line bg-surface p-4"
  >
    <button
      class="flex min-w-0 flex-1 items-center gap-3 text-left"
      onpointerenter={() => seen(item.id)}
      onfocus={() => seen(item.id)}
    >
      <span
        ><strong class="block text-sm">{item.title}</strong><span
          class="mt-1 block text-xs text-muted">{item.state}</span
        ></span
      >
      <AttentionDot
        items={$attention.filter(
          (entry) =>
            entry.id === 'request:' + item.id && entry.target === 'requests',
        )}
      />
    </button>
    {#if item.state === 'available'}
      {@const section =
        item.kind === 'radarr'
          ? 'Movies'
          : item.kind === 'sonarr'
            ? 'Shows'
            : 'Music'}
      <Button size="sm" variant="secondary" onclick={() => navigate(section)}
        >View in {section}</Button
      >
    {/if}
  </article>
{/each}
