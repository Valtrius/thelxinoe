<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { onDestroy, untrack } from 'svelte';
  import { api } from './api';
  import { LatestRequest } from './latest-request';
  import AutoDeletePolicy, {
    type Policy,
    type VideoUsage,
  } from './AutoDeletePolicy.svelte';
  import Button from './ui/Button.svelte';
  let { revision = 0 } = $props<{ revision?: number }>();
  type Settings = {
    policies: Policy[];
    users: { id: string; username: string }[];
    video_usage?: VideoUsage;
  };
  let policies = $state<Policy[]>([]);
  let users = $state<Settings['users']>([]);
  let usage = $state<VideoUsage>({
    unpinned_bytes: 0,
    pinned_bytes: 0,
    waiting: 0,
  });
  let loaded = $state(false);
  let loading = $state(false);
  let error = $state('');
  const requests = new LatestRequest();
  async function refresh() {
    const current = requests.begin();
    loading = true;
    try {
      const result = await api<Settings>('/admin/retention');
      if (!current()) return;
      if (!loaded) {
        policies = result.policies;
        users = result.users;
        loaded = true;
      }
      usage = result.video_usage ?? {
        unpinned_bytes: 0,
        pinned_bytes: 0,
        waiting: 0,
      };
      error = '';
    } catch (failure) {
      if (current()) error = String(failure);
    } finally {
      if (current()) loading = false;
    }
  }
  // Retention and download events bump the settings revision.
  $effect(() => {
    void revision;
    untrack(() => void refresh());
  });
  onDestroy(() => requests.invalidate());
</script>

{#if error}
  <div class="col-span-full flex flex-wrap items-center gap-3">
    <p class="m-0 text-xs text-danger" role="alert">{error}</p>
    <Button variant="secondary" size="form" disabled={loading} onclick={refresh}
      >Retry</Button
    >
  </div>
{/if}
{#if !loaded && loading}
  <ContentSkeleton
    label="Loading retention settings"
    variant="settings"
    class="col-span-full"
  />
{/if}
{#if policies.some((policy) => policy.domain !== 'videos')}
  <!-- Movies and TV seasons are symmetric, so they share a row. -->
  <div class="settings-pair col-span-full">
    {#each policies as policy, i (policy.domain)}
      {#if policy.domain !== 'videos'}
        <AutoDeletePolicy
          bind:policy={policies[i]}
          {users}
          {usage}
          onSave={(value) =>
            api(`/admin/retention/policy/${policy.domain}`, 'POST', value)}
        />
      {/if}
    {/each}
  </div>
{/if}
{#each policies as policy, i (policy.domain)}
  {#if policy.domain === 'videos'}
    <AutoDeletePolicy
      bind:policy={policies[i]}
      {users}
      {usage}
      onSave={(value) =>
        api(`/admin/retention/policy/${policy.domain}`, 'POST', value)}
    />
  {/if}
{/each}
