<script lang="ts">
  import { api } from './api';
  import { serverUpdates } from './server-updates';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  import { formControlClass } from './ui/styles';
  let policy = $state('automatic'),
    start = $state(3),
    end = $state(5);
  let initialized = $state(false);
  $effect(() => {
    if ($serverUpdates.status && !initialized) {
      const saved = $serverUpdates.status.policy;
      policy = saved.policy;
      start = saved.window_start;
      end = saved.window_end;
      initialized = true;
    }
  });
</script>

{#if initialized}
  <AutoSaveForm
    label="Server update settings"
    class="flex flex-wrap items-center gap-x-3 gap-y-2"
    value={{ policy, window_start: start, window_end: end }}
    onRevert={(previous) => {
      policy = previous.policy;
      start = previous.window_start;
      end = previous.window_end;
    }}
    onsave={(submitted) =>
      api('/admin/product-update/policy', 'POST', submitted)}
  >
    {#snippet children(save)}
      <span class="text-xs text-muted">Updates</span>
      <ExclusiveChoiceGroup
        ariaLabel="Update policy"
        value={policy}
        choices={[
          { value: 'notify', label: 'Notify' },
          { value: 'automatic', label: 'Automatic' },
        ]}
        onChange={(value) => {
          policy = value;
          void save();
        }}
      />
      {#if policy === 'automatic'}
        <div class="flex items-center gap-2 text-xs text-muted">
          <label class="flex items-center gap-2"
            >From<input
              aria-label="Maintenance starts"
              class={[formControlClass, 'h-8! w-14! px-2! py-1!']}
              type="number"
              min="0"
              max="23"
              required
              bind:value={start}
            /></label
          >
          <label class="flex items-center gap-2"
            >to<input
              aria-label="Maintenance ends"
              class={[formControlClass, 'h-8! w-14! px-2! py-1!']}
              type="number"
              min="0"
              max="23"
              required
              bind:value={end}
            /></label
          >
          <span>{$serverUpdates.status?.timezone ?? 'UTC'}</span>
        </div>
      {/if}
    {/snippet}
  </AutoSaveForm>
{/if}
