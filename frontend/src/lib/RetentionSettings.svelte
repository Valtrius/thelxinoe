<script lang="ts">
  import ContentSkeleton from './ui/ContentSkeleton.svelte';
  import { onMount } from 'svelte';
  import { api } from './api';
  import AutoDeletePolicy, {
    type Policy,
    type VideoUsage,
  } from './AutoDeletePolicy.svelte';
  import Button from './ui/Button.svelte';
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
  let active = false;
  async function refresh() {
    if (loading) return;
    loading = true;
    try {
      const result = await api<Settings>('/admin/retention');
      if (!active) return;
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
      if (active) error = String(failure);
    } finally {
      if (active) loading = false;
    }
  }
  onMount(() => {
    active = true;
    void refresh();
    const timer = setInterval(() => void refresh(), 30_000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });
</script>

{#if error}
  <div class="col-span-full flex flex-wrap items-center gap-3">
    <p class="text-danger" role="alert">{error}</p>
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
{#each policies as policy, i (policy.domain)}
  <AutoDeletePolicy
    bind:policy={policies[i]}
    {users}
    {usage}
    onSave={(value) =>
      api(`/admin/retention/policy/${policy.domain}`, 'POST', value)}
  />
{/each}
