<script lang="ts">
  import { qrCode } from '../qr';
  let { value, label } = $props<{ value: string; label: string }>();
  const code = $derived(qrCode(value));
</script>

<!-- Scanners expect dark modules on a light field, so these colors deliberately ignore the theme. -->
<svg
  role="img"
  aria-label={label}
  viewBox="0 0 {code.size} {code.size}"
  shape-rendering="crispEdges"
  class="block aspect-square w-full max-w-56 forced-color-adjust-none"
>
  <rect width={code.size} height={code.size} fill="#ffffff" />
  <path d={code.path} fill="#000000" />
</svg>
