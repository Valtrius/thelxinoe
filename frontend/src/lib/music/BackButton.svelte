<script lang="ts">
  import { ArrowLeft } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';

  let { label, back } = $props<{
    /** Names the page Back returns to, when it has a known name. */
    label?: string;
    back: () => void;
  }>();
</script>

<div class="mb-5">
  <Button
    variant="ghost"
    size="sm"
    aria-label={label ? `Back to ${label}` : 'Back'}
    onclick={back}><ArrowLeft size={16} /> {label ?? 'Back'}</Button
  >
</div>
