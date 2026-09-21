<script lang="ts">
  import type { Snippet } from 'svelte';
  import Button from './Button.svelte';
  import { errorClass } from './styles';

  let {
    onsave,
    children,
    disabled = false,
    busy = $bindable(false),
    class: className = '',
    label = 'Preferences',
  } = $props<{
    onsave: () => Promise<unknown>;
    children: Snippet<[() => Promise<void>]>;
    disabled?: boolean;
    busy?: boolean;
    class?: string;
    label?: string;
  }>();
  let form: HTMLFormElement;
  let error = $state(''),
    saved = $state(false);

  async function submit() {
    if (busy || disabled || !form.reportValidity()) return;
    // A single request owns the form until it completes. Keeping edits out of
    // that interval prevents older responses from overwriting newer settings.
    busy = true;
    error = '';
    saved = false;
    try {
      await onsave();
      saved = true;
    } catch (caught) {
      error = String(caught);
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
  <fieldset disabled={disabled || busy} class={['min-w-0', className]}>
    {@render children(submit)}
  </fieldset>
  {#if busy || saved}<p class="mt-2 text-xs text-muted" role="status">
      {busy ? 'Saving…' : 'Saved'}
    </p>{/if}
  {#if error}
    <div class={errorClass} role="alert">
      <p>Your changes could not be saved: {error}</p>
      <Button
        size="form"
        variant="secondary"
        disabled={disabled || busy}
        onclick={() => void submit()}>Retry saving</Button
      >
    </div>
  {/if}
</form>
