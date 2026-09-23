<script lang="ts">
  import ExclusiveChoiceGroup from './ExclusiveChoiceGroup.svelte';
  import FormField from './FormField.svelte';
  import { formControlClass } from './styles';

  let {
    policy = $bindable('notify'),
    start = $bindable(3),
    end = $bindable(5),
    timezone,
    inherited,
    onChange,
  }: {
    policy: string;
    start: number;
    end: number;
    timezone: string;
    inherited?: { policy: string; window_start: number; window_end: number };
    onChange: () => void;
  } = $props();
  const choices = $derived([
    ...(inherited ? [{ value: 'inherit', label: 'Server default' }] : []),
    { value: 'notify', label: 'Notify' },
    { value: 'automatic', label: 'Automatic' },
  ]);
</script>

<div class="grid justify-items-start gap-2">
  <ExclusiveChoiceGroup
    {choices}
    value={policy}
    ariaLabel="Update policy"
    onChange={(next) => {
      policy = next;
      onChange();
    }}
  />
</div>
{#if policy === 'automatic'}
  <div class="grid grid-cols-2 gap-3 compact:grid-cols-1 [&_label]:m-0">
    <FormField
      >Maintenance starts ({timezone})<input
        class={formControlClass}
        type="number"
        min="0"
        max="23"
        required
        bind:value={start}
      /></FormField
    >
    <FormField
      >Maintenance ends ({timezone})<input
        class={formControlClass}
        type="number"
        min="0"
        max="23"
        required
        bind:value={end}
      /></FormField
    >
  </div>
{/if}
