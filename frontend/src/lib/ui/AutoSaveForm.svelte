<script lang="ts" generics="Value extends object">
  import Notice from './Notice.svelte';
  import { onDestroy, untrack, type Snippet } from 'svelte';

  let {
    onsave,
    value,
    onRevert,
    children,
    disabled = false,
    busy = $bindable(false),
    class: className = '',
    label = 'Preferences',
  } = $props<{
    value: Value;
    onsave: (value: Value) => Promise<unknown>;
    onRevert: (value: Value) => void;
    children: Snippet<[() => Promise<void>]>;
    disabled?: boolean;
    busy?: boolean;
    class?: string;
    label?: string;
  }>();
  let form: HTMLFormElement;
  let error = $state('');
  const copy = (value: Value): Value => JSON.parse(JSON.stringify(value));
  let confirmed = copy(untrack(() => value));
  let pending: Value | undefined;
  let active = true;
  onDestroy(() => (active = false));

  async function submit() {
    if (disabled || !form.reportValidity()) return;
    pending = copy(value);
    if (busy) return;
    // Keep the latest edit visible while serializing complete snapshots. An
    // older response must never replace edits made during its request.
    busy = true;
    error = '';
    try {
      while (pending) {
        const submitted = pending;
        pending = undefined;
        try {
          await onsave(submitted);
          confirmed = submitted;
          error = '';
        } catch (caught) {
          if (!pending && active) {
            // Text fields can contain a newer edit before their change/blur
            // event submits it. Preserve that draft when rolling back.
            const reverted = copy(value);
            for (const key of Object.keys(submitted) as (keyof Value)[]) {
              if (
                JSON.stringify(reverted[key]) === JSON.stringify(submitted[key])
              )
                reverted[key] = confirmed[key];
            }
            onRevert(reverted);
            error = String(caught);
          }
        }
      }
    } finally {
      busy = false;
    }
  }
</script>

<form
  bind:this={form}
  aria-label={label}
  onchange={() => void submit()}
  onsubmit={(event) => {
    event.preventDefault();
    void submit();
  }}
>
  <fieldset {disabled} class={['min-w-0', className]}>
    {@render children(submit)}
  </fieldset>
  {#if error}
    <Notice variant="error" role="alert">
      <p>Your changes could not be saved and were reverted: {error}</p>
    </Notice>
  {/if}
</form>
