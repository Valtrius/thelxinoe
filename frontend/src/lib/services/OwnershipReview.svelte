<script lang="ts">
  import type { TransferReview } from './feature';
  import Switch from '../ui/Switch.svelte';
  import Notice from '../ui/Notice.svelte';

  let { review, releasedCompose = $bindable(false) } = $props<{
    review: TransferReview;
    releasedCompose?: boolean;
  }>();
</script>

<section class="work-section settings-section" aria-label="Ownership review">
  <h3 class="mb-3.25 text-[12px] font-[650]">Ownership review</h3>
  <dl
    class="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] gap-x-4 gap-y-2 text-[11px]"
  >
    {#each [['Container', 'Existing container retained'], ['Application configuration', 'Unchanged'], ['Storage and ports', 'Unchanged'], ['Authentication', 'Unchanged'], ['Restart required', review.restart_required ? 'Yes' : 'No'], ['Updates', 'Check only'], ['Configuration location', review.source_config ?? 'No /config mount'], ['Retained image', review.image]] as [label, value] (label)}
      <dt class="text-muted">{label}</dt>
      <dd class="min-w-0 wrap-anywhere">{value}</dd>
    {/each}
  </dl>
  {#each ['lifecycle', 'backup', 'update'] as capability (capability)}
    {@const status =
      review.capabilities[capability as 'lifecycle' | 'backup' | 'update']}
    {#if !status.available && status.reason}
      <Notice tone="warning" class="mt-3 text-[11px]" role="status"
        >{status.reason}</Notice
      >
    {/if}
  {/each}
  {#each review.automation ?? [] as warning (warning)}
    <Notice tone="warning" class="mt-3 text-[11px]" role="status"
      >{warning}</Notice
    >
  {/each}
  {#if review.integration_ready === false}
    <p class="mt-3 text-[11px] text-muted">
      {review.integration_error ??
        'The container is stopped. API access remains unverified until you start it.'}
    </p>
  {/if}
  {#if review.warnings?.length}
    <div class="mt-3 text-[11px]">
      <strong>Security recommendations</strong>
      <ul class="mt-1 list-disc space-y-1 pl-4 text-muted">
        {#each review.warnings as warning (warning)}<li>{warning}</li>{/each}
      </ul>
      <p class="mt-2 text-muted">
        Review these in the service's native settings. Taking ownership
        preserves them.
      </p>
    </div>
  {/if}
  {#if review.compose_project}
    <p class="my-3 text-[11px] leading-[1.6] text-muted">
      Retire automation for the entire Compose project <strong
        >{review.compose_project}</strong
      >. A continuing project can remove this container with
      <code>--remove-orphans</code>. Detachment from an active project requires
      a separately reviewed recreation.
    </p>
    <Switch size="sm" bind:checked={releasedCompose}
      >The entire previous Compose project is retired</Switch
    >
  {:else}
    <p class="mt-3 text-[11px] leading-[1.6] text-muted">
      Disable any external deployment automation or updater before taking
      ownership.
    </p>
  {/if}
</section>
