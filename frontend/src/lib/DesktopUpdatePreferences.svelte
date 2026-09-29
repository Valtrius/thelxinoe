<script lang="ts">
  import { desktopUpdates, desktopUpdate } from './desktop-updates';
  import AutoSaveForm from './ui/AutoSaveForm.svelte';
  import ExclusiveChoiceGroup from './ui/ExclusiveChoiceGroup.svelte';
  let policy = $state<'notify' | 'automatic'>('notify');
  let initialized = $state(false);
  $effect(() => {
    if ($desktopUpdates && !initialized) {
      policy = $desktopUpdates.policy;
      initialized = true;
    }
  });
</script>

{#if initialized}
  <AutoSaveForm
    label="Desktop update settings"
    class="flex items-center gap-3"
    value={{ policy }}
    onRevert={(previous) => (policy = previous.policy)}
    onsave={(value) => desktopUpdate('policy', value)}
  >
    {#snippet children(save)}
      <span class="text-xs text-muted">Updates</span>
      <ExclusiveChoiceGroup
        ariaLabel="Desktop update policy"
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
    {/snippet}
  </AutoSaveForm>
{/if}
