<script lang="ts">
  import { api } from '../api';
  import type { Container } from './presentation';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  let {
    containers,
    install,
    changed,
  }: {
    containers: Container[];
    install: () => Promise<void>;
    changed: () => Promise<void>;
  } = $props();
  let container = $state(''),
    pending = $state<'install' | 'connect' | null>(null),
    error = $state('');
  async function work(
    kind: 'install' | 'connect',
    action: () => Promise<void>,
  ) {
    if (pending) return;
    pending = kind;
    error = '';
    try {
      await action();
    } catch (e) {
      error = String(e);
    } finally {
      pending = null;
    }
  }
</script>

<div class="grid gap-5.5">
  <div class="grid gap-3">
    <strong class="text-xs">Install and own it</strong>
    <Button
      size="form"
      variant="secondary"
      class="w-fit"
      loading={pending === 'install'}
      disabled={!!pending}
      onclick={() => void work('install', install)}>Install Recyclarr</Button
    >
  </div>
  <form
    class="grid gap-3 border-t border-line pt-3 [&_label]:m-0"
    aria-label="Connect existing Recyclarr"
    onsubmit={(event) => {
      event.preventDefault();
      void work('connect', async () => {
        const result = await api<{ healthy: boolean; error: string | null }>(
          '/admin/recyclarr/connection',
          'POST',
          { container_id: container },
        );
        await changed();
        // The job is connected even when validation found something to fix.
        if (!result.healthy && result.error) error = result.error;
      });
    }}
  >
    <strong class="text-xs">Connect an existing job container</strong>
    <p class="text-[11px] text-muted">
      Recyclarr has no web API. Thelxinoe validates its configuration and
      container read-only; nothing runs and no schedule changes.
    </p>
    <FormField
      >Container<select
        class={formControlClass}
        bind:value={container}
        required
      >
        <option value="">Select container</option>
        {#each containers as item (item.id)}<option value={item.id}
            >{item.names[0]}</option
          >{/each}
      </select></FormField
    >
    <Button
      type="submit"
      size="form"
      class="w-fit"
      loading={pending === 'connect'}
      disabled={!!pending || !container}>Validate and connect</Button
    >
  </form>
  {#if error}<Notice tone="danger" role="alert">{error}</Notice>{/if}
</div>
