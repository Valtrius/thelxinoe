<script lang="ts">
  import { SquareCheck } from '@lucide/svelte';
  import Button from './Button.svelte';

  let {
    test,
    onError,
    succeeded = $bindable(false),
    disabled = false,
    label = 'Test connection',
    size = 'form',
  } = $props<{
    test: () => Promise<boolean | void>;
    onError: (error: unknown) => void;
    succeeded?: boolean;
    disabled?: boolean;
    label?: string;
    size?: 'form' | 'sm';
  }>();
  let running = $state(false);
  $effect(() => {
    if (!succeeded) return;
    const timer = setTimeout(() => (succeeded = false), 1000);
    return () => clearTimeout(timer);
  });
  async function run() {
    if (running || disabled) return;
    running = true;
    succeeded = false;
    try {
      succeeded = (await test()) !== false;
    } catch (error) {
      onError(error);
    } finally {
      running = false;
    }
  }
</script>

<Button
  variant="secondary"
  {size}
  class="relative"
  aria-label={running
    ? 'Testing connection'
    : succeeded
      ? 'Connection successful'
      : label}
  loading={running}
  {disabled}
  onclick={() => void run()}
>
  <span class:invisible={succeeded}>{label}</span>
  {#if succeeded}
    <span
      class="absolute inset-0 grid place-items-center text-accent"
      role="status"
      aria-label="Connection successful"><SquareCheck size={18} /></span
    >
  {/if}
</Button>
