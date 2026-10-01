<script lang="ts">
  import { api } from '../api';
  import type { Container } from './presentation';
  import Modal from '../ui/Modal.svelte';
  import Button from '../ui/Button.svelte';
  import FormField from '../ui/FormField.svelte';
  import Notice from '../ui/Notice.svelte';
  import Switch from '../ui/Switch.svelte';
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
  let mode = $state<'import' | null>(null),
    container = $state(''),
    busy = $state(false),
    error = $state(''),
    confirmed = $state(false);
  let review = $state<{
    review_id: string;
    targets: {
      service_id: string;
      name: string;
      kind: string;
      trash_id: string;
      guide_name: string;
    }[];
  } | null>(null);
  async function work(action: () => Promise<void>) {
    busy = true;
    error = '';
    try {
      await action();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="flex flex-wrap gap-3">
  <Button
    size="form"
    variant="secondary"
    loading={busy && !mode}
    onclick={() => void work(install)}>Install Recyclarr</Button
  >
  <Button
    size="form"
    variant="secondary"
    disabled={busy}
    onclick={() => {
      error = '';
      review = null;
      confirmed = false;
      mode = 'import';
    }}>Import existing Recyclarr</Button
  >
</div>
{#if error && !mode}<Notice tone="danger" role="alert">{error}</Notice>{/if}
{#if mode}
  <Modal
    title="Import Recyclarr"
    {busy}
    onClose={() => {
      mode = null;
    }}
  >
    <div class="grid gap-4 text-xs">
      {#if error}<Notice tone="danger" role="alert">{error}</Notice>{/if}
      <FormField
        >Existing container<select
          class={formControlClass}
          value={container}
          onchange={(e) => {
            container = e.currentTarget.value;
            review = null;
            confirmed = false;
          }}
        >
          <option value="">Select container</option>
          {#each containers as item (item.id)}<option value={item.id}
              >{item.names[0]}</option
            >{/each}
        </select></FormField
      >
      <p class="text-muted">
        The original scheduler stops, its appdata is copied, and the managed
        schedule starts paused.
      </p>
      <Button
        size="form"
        variant="secondary"
        disabled={busy || !container}
        onclick={() =>
          void work(async () => {
            review = await api('/admin/recyclarr/adopt/preview', 'POST', {
              container_id: container,
            });
          })}>Review import</Button
      >
      {#if review}
        {#each review.targets as target (`${target.service_id}:${target.trash_id}`)}<p
          >
            {target.name}: {target.guide_name}
          </p>{/each}
        <Switch bind:checked={confirmed}
          >I have disabled external schedulers and released this container from
          its external manager.</Switch
        >
        <Button
          size="form"
          disabled={busy || !confirmed}
          onclick={() =>
            void work(async () => {
              await api('/admin/recyclarr/adopt', 'POST', {
                review_id: review!.review_id,
                container_id: container,
                released_compose: confirmed,
              });
              await changed();
              mode = null;
            })}>Transfer ownership</Button
        >
      {/if}
    </div>
  </Modal>
{/if}
