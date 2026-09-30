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
  let mode = $state<'install' | 'import' | null>(null),
    container = $state(''),
    busy = $state(false),
    error = $state(''),
    confirmed = $state(false);
  let review = $state<{
    review_id: string;
    targets: {
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
    onclick={() => {
      error = '';
      mode = 'install';
    }}>Install Recyclarr</Button
  >
  <Button
    size="form"
    variant="secondary"
    onclick={() => {
      error = '';
      review = null;
      confirmed = false;
      mode = 'import';
    }}>Import existing Recyclarr</Button
  >
</div>
{#if mode}
  <Modal
    title={mode === 'install' ? 'Install Recyclarr' : 'Import Recyclarr'}
    {busy}
    onClose={() => {
      mode = null;
    }}
  >
    <div class="grid gap-4 text-xs">
      {#if error}<Notice tone="danger" role="alert">{error}</Notice>{/if}
      {#if mode === 'install'}
        <p>
          Install the official Docker job service. Connected Radarr instances
          use HD Bluray + WEB; Sonarr instances use WEB-1080p. Initial and daily
          syncs apply custom formats, scores and quality profiles automatically.
        </p>
        <p class="text-muted">
          Profile selection changes defaults for new additions. Existing movies
          and series keep their assigned profiles. Global quality-size limits
          are opt-in.
        </p>
        <Button
          size="form"
          disabled={busy}
          onclick={() =>
            void work(async () => {
              await install();
              mode = null;
            })}>Install</Button
        >
      {:else}
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
          Supports one official guide profile per connected Radarr or Sonarr
          instance. The original scheduler stops, its appdata is copied, and the
          managed schedule starts paused.
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
          {#each review.targets as target (target.name)}<p>
              {target.name}: {target.guide_name}
            </p>{/each}
          <Switch bind:checked={confirmed}
            >I have disabled external schedulers and released this container
            from its external manager.</Switch
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
      {/if}
    </div>
  </Modal>
{/if}
