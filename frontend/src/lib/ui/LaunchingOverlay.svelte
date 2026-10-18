<script lang="ts">
  import { LoaderCircle } from '@lucide/svelte';
  let { label } = $props<{ label: string }>();
</script>

<!-- Covers artwork while its video starts in a desktop MPV window. Later
  siblings, such as badges, stay above it. -->
<span
  role="status"
  aria-label={label}
  class="pointer-events-none absolute inset-0 grid place-items-center bg-black/45 text-white"
  data-launching
>
  <LoaderCircle
    size={32}
    class="animate-spin motion-reduce:animate-none"
    aria-hidden="true"
  />
</span>
