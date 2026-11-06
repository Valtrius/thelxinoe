<script lang="ts">
  import { fade } from 'svelte/transition';
  import { ArrowLeft } from '@lucide/svelte';
  import { getAnimationDuration } from '../motion';
  import Button from '../ui/Button.svelte';

  let { label, back } = $props<{
    /** Names the page Back returns to, when it has a known name. */
    label?: string;
    back: () => void;
  }>();
</script>

<div
  in:fade={{ duration: getAnimationDuration() }}
  class="flex h-9 items-center"
>
  <Button
    variant="ghost"
    size="sm"
    aria-label={label ? `Back to ${label}` : 'Back'}
    onclick={back}><ArrowLeft size={16} /> {label ?? 'Back'}</Button
  >
</div>
