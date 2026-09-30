<script lang="ts">
  import type { Guide, Target } from './recyclarr';
  import { api } from '../api';
  import AutoSaveForm from '../ui/AutoSaveForm.svelte';
  import FormField from '../ui/FormField.svelte';
  import Switch from '../ui/Switch.svelte';
  import Notice from '../ui/Notice.svelte';
  import { formControlClass } from '../ui/styles';
  let {
    target,
    guides,
    changed,
  }: { target: Target; guides: Guide[]; changed: () => Promise<void> } =
    $props();
  const initial = () => ({
    trash_id: target.trash_id,
    quality_sizes: target.quality_sizes,
    reset_scores: target.reset_scores,
    groups: target.groups,
    overrides: target.overrides,
  });
  let selection = $state(initial());
  const guide = $derived(guides.find((g) => g.trash_id === selection.trash_id));
</script>

<div class="grid gap-3 border-t border-line pt-4">
  <strong class="text-xs">{target.name}</strong>
  <AutoSaveForm
    label={`${target.name} guide profile`}
    value={selection}
    class="grid gap-3"
    onsave={async (value) => {
      await api(`/admin/recyclarr/targets/${target.service_id}`, 'POST', value);
      await changed();
    }}
    onRevert={(value) => {
      selection = value;
    }}
  >
    <FormField
      >Guide profile<select
        aria-label={`Guide profile for ${target.name}`}
        class={formControlClass}
        value={selection.trash_id}
        onchange={(event) => {
          selection = {
            ...selection,
            trash_id: event.currentTarget.value,
            groups: { add: [], skip: [] },
            overrides: {},
          };
        }}
      >
        {#if !guide}<option value={selection.trash_id}
            >{guides.length
              ? 'Selected guide unavailable'
              : 'Loading guide profiles…'}</option
          >{/if}
        {#each guides as item (item.trash_id)}<option value={item.trash_id}
            >{item.name}</option
          >{/each}
      </select></FormField
    >
    <details class="text-[11px]">
      <summary class="cursor-pointer text-muted">Guide options</summary>
      <div class="grid gap-3 pt-3">
        <Switch size="sm" bind:checked={selection.reset_scores}
          >Reset unmatched scores in this guide profile</Switch
        >
        <Switch size="sm" bind:checked={selection.quality_sizes}
          >Apply global quality-size limits</Switch
        >
        <p class="text-[10px] leading-4 text-muted">
          Quality-size limits affect every profile in this instance. Naming and
          media-management settings are unchanged.
        </p>
        {#each guide?.groups ?? [] as group (group.trash_id)}
          <FormField
            >{group.name}<select
              class={formControlClass}
              value={selection.groups.skip.includes(group.trash_id)
                ? 'skip'
                : selection.groups.add.some(
                      (g) => g.trash_id === group.trash_id,
                    )
                  ? 'add'
                  : 'guide'}
              onchange={(event) => {
                const mode = event.currentTarget.value;
                selection.groups = {
                  add: [
                    ...selection.groups.add.filter(
                      (g) => g.trash_id !== group.trash_id,
                    ),
                    ...(mode === 'add' ? [{ trash_id: group.trash_id }] : []),
                  ],
                  skip: [
                    ...selection.groups.skip.filter(
                      (id) => id !== group.trash_id,
                    ),
                    ...(mode === 'skip' ? [group.trash_id] : []),
                  ],
                };
              }}
            >
              <option value="guide">Guide default</option><option value="add"
                >Include</option
              ><option value="skip">Exclude</option>
            </select></FormField
          >
        {/each}
        {#each ['min_format_score', 'upgrade_until_score'] as field (field)}
          <FormField
            >{field === 'min_format_score'
              ? 'Minimum custom-format score'
              : 'Upgrade until score'}<input
              class={formControlClass}
              type="number"
              min="-100000"
              max="100000"
              placeholder="Guide default"
              value={selection.overrides[field] as number | undefined}
              oninput={(event) => {
                const values = { ...selection.overrides };
                if (event.currentTarget.value === '') delete values[field];
                else values[field] = Number(event.currentTarget.value);
                selection.overrides = values;
              }}
            /></FormField
          >
        {/each}
        <FormField
          >Upgrades<select
            class={formControlClass}
            value={selection.overrides.upgrade_allowed === undefined
              ? 'guide'
              : String(selection.overrides.upgrade_allowed)}
            onchange={(event) => {
              const values = { ...selection.overrides };
              if (event.currentTarget.value === 'guide')
                delete values.upgrade_allowed;
              else
                values.upgrade_allowed = event.currentTarget.value === 'true';
              selection.overrides = values;
            }}
          >
            <option value="guide">Guide default</option><option value="true"
              >Allow upgrades</option
            ><option value="false">Disable upgrades</option>
          </select></FormField
        >
      </div>
    </details>
  </AutoSaveForm>
  {#if target.error}<Notice tone="danger" role="alert">{target.error}</Notice
    >{/if}
  {#if target.profile_name}<span class="text-[10px] text-muted"
      >Applied profile: {target.profile_name}</span
    >{/if}
  {#if guide?.url}<a
      class="text-[10px] text-accent hover:underline"
      href={guide.url}
      target="_blank"
      rel="noreferrer">Guide details</a
    >{/if}
</div>
