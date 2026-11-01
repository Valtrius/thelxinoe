<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../api';
  import { LatestRequest } from '../latest-request';
  import { inBackground } from '../page-readiness';
  import { settingsPages } from '../pages';
  import { isServerUpdateInterruption } from '../server-updates';
  import Notice from '../ui/Notice.svelte';
  import SettingsCard from '../ui/SettingsCard.svelte';
  import Skeleton from '../ui/Skeleton.svelte';
  import { cardBlockClass, statGridClass, statTileClass } from '../ui/styles';

  let { revision } = $props<{ revision: number }>();
  type Health = {
    version: string;
    controller: boolean;
    cache_free_bytes: number;
  };
  let health = $state<Health | null>(null),
    loaded = $state(false),
    error = $state('');
  const requests = new LatestRequest();
  onDestroy(() => requests.invalidate());
  $effect(() => {
    void revision;
    untrack(() => void load());
  });
  async function load() {
    const latest = requests.begin();
    try {
      // Health waits on the Docker controller; the tiles show their own loading.
      const value = await inBackground(() => api<Health>('/admin/health'));
      if (!latest()) return;
      health = value;
      error = '';
    } catch (caught) {
      if (latest() && !isServerUpdateInterruption(caught))
        error = String(caught);
    } finally {
      if (latest()) loaded = true;
    }
  }
</script>

<SettingsCard
  id="server-status"
  aria-label="Server status"
  title="Server status"
  description="Installed version, update availability and controller health."
>
  <div class={statGridClass}>
    <div class={statTileClass}>
      {#await settingsPages.ProductVersion()}<Skeleton
          class="h-6 w-24"
        />{:then { default: ProductVersion }}<ProductVersion
          installed={health?.version ?? '—'}
        />{:catch caught}<Notice variant="error" role="alert" class="mb-0"
          >{String(caught)}</Notice
        >{/await}
    </div>
    <div class={statTileClass}>
      {#if !loaded}<Skeleton class="h-6 w-24" />{:else}<strong
          >{health?.controller ? 'Connected' : 'Unavailable'}</strong
        >{/if}
      <small>Docker controller</small>
    </div>
    <div class={statTileClass}>
      {#if !loaded}<Skeleton class="h-6 w-24" />{:else}<strong
          >{health
            ? `${(health.cache_free_bytes / 1024 ** 3).toFixed(1)} GB`
            : '—'}</strong
        >{/if}
      <small>Cache space available</small>
    </div>
  </div>
  {#if error}<div class={cardBlockClass}>
      <Notice variant="error" role="alert" class="mb-0">{error}</Notice>
    </div>{/if}
</SettingsCard>
