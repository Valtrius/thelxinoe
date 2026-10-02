<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { FileCode } from '@lucide/svelte';
  import { api } from '../api';
  import Button from '../ui/Button.svelte';
  import type {
    RecyclarrConfiguration,
    YamlFiles,
  } from './recyclarr-configuration';
  let { provisionId } = $props<{ provisionId: string }>();
  let configuration = $state<RecyclarrConfiguration | null>(null),
    error = $state(''),
    opening = $state(false);
  let editor = $state(false),
    review = $state(false),
    candidate = $state(false),
    draft = $state<YamlFiles | undefined>();
  let Editor = $state<
    (typeof import('./RecyclarrYamlModal.svelte'))['default'] | null
  >(null);
  let Review = $state<
    (typeof import('./RecyclarrConfigurationReview.svelte'))['default'] | null
  >(null);
  let alive = true;
  let opener: HTMLElement | null = null;
  function close() {
    editor = false;
    review = false;
    void tick().then(() => {
      if (opener?.isConnected) opener.focus();
    });
    void refresh();
  }
  async function refresh() {
    try {
      const result = await api<RecyclarrConfiguration>(
        '/admin/recyclarr/configuration',
      );
      if (alive && !editor && !review) {
        configuration = result;
        error = '';
      }
    } catch (caught) {
      if (alive) error = String(caught);
    }
  }
  onMount(() => {
    void refresh();
    const timer = setInterval(() => {
      if (!editor && !review) void refresh();
    }, 15000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  });
  async function open(
    mode: 'editor' | 'defaults' | 'candidate',
    files?: YamlFiles,
    candidateDraft = false,
  ) {
    const standalone = !editor && !review;
    if (standalone && document.activeElement instanceof HTMLElement)
      opener = document.activeElement;
    opening = true;
    error = '';
    try {
      if (standalone) await refresh();
      if (!configuration || error) return;
      candidate = mode === 'candidate' || candidateDraft;
      draft = files;
      if (mode === 'editor') {
        Editor = (await import('./RecyclarrYamlModal.svelte')).default;
        review = false;
        editor = true;
      } else {
        Review = (await import('./RecyclarrConfigurationReview.svelte'))
          .default;
        editor = false;
        review = true;
      }
    } catch (caught) {
      error = String(caught);
    } finally {
      opening = false;
    }
  }
</script>

<div class="grid gap-2 border-t border-line pt-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h3 class="text-xs font-semibold">Configuration</h3>
      <p class="mt-1 text-xs text-muted">
        {configuration
          ? configuration.mode === 'defaults'
            ? 'Defaults'
            : 'Customized'
          : 'Loading configuration…'}
      </p>
    </div>
    <div class="flex flex-wrap gap-2">
      {#if configuration?.candidate}<Button
          size="form"
          variant="secondary"
          onclick={() => void open('candidate')}
          >Review update configuration</Button
        >{/if}{#if configuration?.mode === 'customized'}<Button
          size="form"
          variant="secondary"
          onclick={() => void open('defaults')}>Review defaults</Button
        >{/if}<Button
        size="form"
        loading={opening}
        onclick={() => void open('editor')}
        ><FileCode size={14} />Edit YAML</Button
      >
    </div>
  </div>
  {#if configuration?.mode === 'defaults'}<p class="text-[11px] text-muted">
      Defaults update with Recyclarr upgrades.
    </p>{/if}
  {#if error}<p class="text-xs text-danger" role="alert">
      {error}
      <button
        type="button"
        class="underline underline-offset-2"
        onclick={() => void refresh()}>Retry</button
      >
    </p>{/if}
</div>
{#if editor && Editor && configuration}<Editor
    initial={configuration}
    {draft}
    {candidate}
    onClose={close}
    onUpdate={(value) => (configuration = value)}
  />{/if}
{#if review && Review && configuration}<Review
    {configuration}
    {candidate}
    {provisionId}
    onClose={close}
    onUpdate={(value) => (configuration = value)}
    onEdit={(files, candidate) => void open('editor', files, candidate)}
  />{/if}
