<script lang="ts">
  import {
    providers,
    providerNames,
    saveProvider,
    type Provider,
  } from './availability';
  import Switch from '../ui/Switch.svelte';
  let { provider }: { provider: Provider } = $props();
  let enabled = $state(true);
  let busy = $state(false);
  let error = $state('');
  $effect(() => {
    if (!busy) enabled = $providers[provider];
  });
  async function save(checked: boolean) {
    enabled = checked;
    busy = true;
    error = '';
    try {
      await saveProvider(provider, checked);
    } catch (caught) {
      error = String(caught);
    } finally {
      enabled = $providers[provider];
      busy = false;
    }
  }
</script>

<!-- Sits in a card header: a visible state word plus the named switch. -->
<div class="flex flex-wrap items-center justify-end gap-2">
  {#if error}<span role="alert" class="text-[11px] text-danger">{error}</span
    >{/if}
  <span class="text-[11px] text-muted" aria-hidden="true"
    >{enabled ? 'Enabled' : 'Disabled'}</span
  >
  <Switch
    checked={enabled}
    disabled={busy}
    onCheckedChange={(checked) => void save(checked)}
    ><span class="sr-only">Enable {providerNames[provider]}</span></Switch
  >
</div>
