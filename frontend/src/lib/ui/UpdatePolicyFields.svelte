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
  <span class="text-[11px] font-[550]">Update policy</span>
  <ExclusiveChoiceGroup
    {choices}
    value={policy}
    ariaLabel="Update policy"
    onChange={(next) => {
      policy = next;
      onChange();
    }}
  />
  <p class="text-xs text-muted">
    Checks automatically every six hours. Notify lets you choose when to
    install. Automatic installs during the maintenance window when the service
    is idle.
  </p>
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
{:else if policy === 'inherit' && inherited}
  <p class="text-xs text-muted">
    Uses the server policy: {inherited.policy === 'automatic'
      ? 'Automatic'
      : 'Notify'}{#if inherited.policy === 'automatic'},
      {String(inherited.window_start).padStart(2, '0')}:00–{String(
        inherited.window_end,
      ).padStart(2, '0')}:00 ({timezone}){/if}.
  </p>
{/if}
