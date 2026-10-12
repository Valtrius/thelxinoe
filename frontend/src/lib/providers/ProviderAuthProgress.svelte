<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';

  let { label }: { label: string } = $props();
</script>

<p
  role="status"
  class="mt-5 flex items-center justify-center gap-2 text-sm text-muted"
>
  <LoaderCircle
    aria-hidden="true"
    class="size-4 shrink-0 animate-spin motion-reduce:animate-none"
  />
  {label}
</p>
