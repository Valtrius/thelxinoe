<script lang="ts">
  import FormField from './ui/FormField.svelte';
  import { formControlClass } from './ui/styles';
  import { onMount } from 'svelte';
  import { api } from './api';
  import Button from './ui/Button.svelte';

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

<FormField>
  {label}
  <select
    class={formControlClass}
    bind:value
    disabled={disabled || loading || Boolean(error)}
  >
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
</FormField>
{#if error}
  <p role="alert">Could not load timezones: {error}</p>
  <Button
    type="button"
    variant="secondary"
    size="form"
    onclick={() => void load()}>Retry loading timezones</Button
  >
{/if}
