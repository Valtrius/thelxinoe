<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  type Props = Omit<HTMLInputAttributes, 'type' | 'value'> & {
    /** Where playback is; shown whenever the pointer isn't holding the thumb. */
    position: number;
    onseek: (to: number) => void;
    /** The held value while dragging, for labels that follow the thumb. */
    scrub?: number | null;
  };
  // The bar keeps the value under the pointer until it is released; playback
  // updates would otherwise replace it and seek back on release. Releasing where
  // the drag began fires no change event, so release clears it.
  let { position, onseek, scrub = $bindable(null), ...rest }: Props = $props();
</script>

<input
  {...rest}
  type="range"
  value={scrub ?? position}
  oninput={(event) => (scrub = Number(event.currentTarget.value))}
  onchange={(event) => {
    scrub = null;
    onseek(Number(event.currentTarget.value));
  }}
  onpointerup={() => setTimeout(() => (scrub = null))}
  onpointercancel={() => (scrub = null)}
/>
