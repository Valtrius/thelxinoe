<script lang="ts">
  import { Ellipsis } from '@lucide/svelte';
  import Button from '../ui/Button.svelte';
  import Modal from '../ui/Modal.svelte';
  import MediaOperations from '../MediaOperations.svelte';
  let {
    id,
    kind,
    title,
    changed,
  }: {
    id: string;
    kind: string;
    title: string;
    changed: () => void;
  } = $props();
  let open = $state(false);
</script>

<Button
  variant="secondary"
  size="icon"
  aria-label={`Manage ${title}`}
  title="Manage media"
  onclick={() => (open = true)}><Ellipsis size={16} /></Button
>
{#if open}<Modal title={`Manage ${title}`} onClose={() => (open = false)}
    ><MediaOperations {id} {kind} {changed} /></Modal
  >{/if}
