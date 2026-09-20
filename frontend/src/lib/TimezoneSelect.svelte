<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from './api';

  let {
    label,
    value = $bindable('UTC'),
    defaultTimezone,
    disabled = false,
  }: {
    label: string;
    value?: string;
    defaultTimezone?: string;
    disabled?: boolean;
  } = $props();
  let zones = $state<string[]>([]),
    loading = $state(true),
    error = $state('');

  async function load() {
    loading = true;
    error = '';
    try {
      zones = (await api<{ timezones: string[] }>('/timezones')).timezones;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => void load());
</script>

<label>
  {label}
  <select bind:value disabled={disabled || loading || Boolean(error)}>
    {#if defaultTimezone}
      <option value="">Use server default ({defaultTimezone})</option>
    {/if}
    {#if !zones.length && value}
      <option {value}>{value}</option>
    {/if}
    {#each zones as zone (zone)}
      <option value={zone}>{zone}</option>
    {/each}
  </select>
</label>
{#if error}
  <p role="alert">Could not load timezones: {error}</p>
  <button type="button" class="secondary" onclick={() => void load()}
    >Retry loading timezones</button
  >
{/if}
