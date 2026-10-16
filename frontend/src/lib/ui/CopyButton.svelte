<script lang="ts">
  import { Check, Copy } from '@lucide/svelte';
  import { copyText } from '../clipboard';
  import Button from './Button.svelte';

  // Icon button that copies `value` and briefly confirms with a check mark.
  let {
    value,
    label,
    disabled = false,
    class: className = '',
  }: {
    value: string;
    label: string;
    disabled?: boolean;
    class?: string;
  } = $props();
  let copied = $state(false);
  let failed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  async function copy() {
    clearTimeout(timer);
    failed = false;
    try {
      await copyText(value);
      copied = true;
    } catch {
      failed = true;
    }
    timer = setTimeout(() => (copied = failed = false), 1500);
  }
</script>

<Button
  variant="secondary"
  size="form"
  class={`shrink-0 px-2.5 ${className}`}
  aria-label={copied ? 'Copied' : failed ? 'Copy failed' : label}
  title={label}
  {disabled}
  onclick={() => void copy()}
>
  {#if copied}<Check
      size={14}
      class="text-success"
      aria-hidden="true"
    />{:else}<Copy size={14} aria-hidden="true" />{/if}
  <span class="sr-only" role="status"
    >{copied ? 'Copied' : failed ? 'Copy failed' : ''}</span
  >
</Button>
