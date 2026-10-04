<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../api';
  import {
    attention,
    acknowledgeAttention,
    type RequestAttention,
  } from '../attention';
  import AttentionDot from '../ui/AttentionDot.svelte';
  import Button from '../ui/Button.svelte';
  import ContentSkeleton from '../ui/ContentSkeleton.svelte';
  let { navigate } = $props<{ navigate: (section: string) => void }>();
  type RequestRow = {
    id: string;
    title: string;
    kind: string;
    state: string;
    attention?: RequestAttention | null;
  };
  let items = $state<RequestRow[]>([]);
  let error = $state('');
  let page = $state(1),
    pages = $state(1),
    loading = $state(false);
  let active = true,
    generation = 0,
    requestedPage = 1;
  const requestAttention = $derived(
    $attention
      .filter((item) => item.id.startsWith('request:'))
      .map((item) => `${item.id}:${item.revision}`)
      .join('\n'),
  );
  $effect(() => {
    void requestAttention;
    untrack(() => void load(requestedPage));
  });
  onDestroy(() => {
    active = false;
    generation++;
  });
  async function load(nextPage: number) {
    requestedPage = nextPage;
    const version = ++generation;
    loading = true;
    error = '';
    try {
      const result = await api<{
        items: RequestRow[];
        page: number;
        pages: number;
      }>(`/acquisition/requests?page=${nextPage}`);
      if (!active || version !== generation) return;
      items = result.items;
      page = result.page;
      requestedPage = result.page;
      pages = result.pages;
    } catch {
      if (active && version === generation)
        error = 'Could not load request history.';
    } finally {
      if (active && version === generation) loading = false;
    }
  }
  function markers(row: RequestRow) {
    return $attention.filter(
      (entry) =>
        entry.target === 'requests' &&
        entry.id === row.attention?.id &&
        entry.revision === row.attention.revision,
    );
  }
  function seen(row: RequestRow) {
    for (const item of markers(row)) void acknowledgeAttention(item);
  }
</script>

{#if error}<div class="mt-4 flex items-center gap-3">
    <p role="alert" class="text-sm text-danger">{error}</p>
    <Button variant="ghost" size="sm" onclick={() => void load(page)}
      >Try again</Button
    >
  </div>{/if}
{#if loading && !items.length}<ContentSkeleton
    label="Loading request history"
  />{/if}
{#each items as item (item.id)}
  <article
    class="mt-3 flex items-center gap-4 border border-line bg-surface p-4"
  >
    <button
      class="flex min-w-0 flex-1 items-center gap-3 text-left"
      onpointerenter={() => seen(item)}
      onfocus={() => seen(item)}
    >
      <span
        ><strong class="block text-sm">{item.title}</strong><span
          class="mt-1 block text-xs text-muted">{item.state}</span
        ></span
      >
      <AttentionDot items={markers(item)} />
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
{#if pages > 1}
  <nav
    aria-label="Acquisition request pages"
    class="mt-5 flex items-center justify-center gap-4"
  >
    <Button
      variant="secondary"
      size="sm"
      disabled={page <= 1 || loading}
      onclick={() => void load(page - 1)}>Newer requests</Button
    >
    <span class="text-xs text-muted">{page} / {pages}</span>
    <Button
      variant="secondary"
      size="sm"
      disabled={page >= pages || loading}
      onclick={() => void load(page + 1)}>Older requests</Button
    >
  </nav>
{/if}
