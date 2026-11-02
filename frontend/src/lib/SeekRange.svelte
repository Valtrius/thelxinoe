<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import { time, type Trickplay } from './playback';

  type Props = Omit<HTMLInputAttributes, 'type' | 'value'> & {
    /** Where playback is; shown whenever the pointer isn't holding the thumb. */
    position: number;
    onseek: (to: number) => void;
    /** The held value while dragging, for labels that follow the thumb. */
    scrub?: number | null;
    /** Thumbnails shown above the bar where a mouse hovers or a drag holds it. */
    preview?: Trickplay | null;
  };
  // The bar keeps the value under the pointer until it is released; playback
  // updates would otherwise replace it and seek back on release. Releasing where
  // the drag began fires no change event, so release clears it.
  let {
    position,
    onseek,
    scrub = $bindable(null),
    preview = null,
    ...rest
  }: Props = $props();
  let input = $state<HTMLInputElement>();
  let hover = $state<number | null>(null);
  let previewWidth = $state(0);
  const min = $derived(Number(rest.min ?? 0));
  const max = $derived(Number(rest.max ?? 100));
  const shown = $derived(
    preview?.sheets.length ? (scrub ?? hover ?? null) : null,
  );
  // The thumb's centre travels inset by half its width, as the bar maps the
  // pointer to a value.
  function thumb() {
    return input
      ? parseFloat(getComputedStyle(input).getPropertyValue('--range-thumb')) ||
          0
      : 0;
  }
  function hovered(event: PointerEvent) {
    if (event.pointerType === 'touch' || !input) return;
    const box = input.getBoundingClientRect();
    const size = thumb();
    const fraction = Math.min(
      1,
      Math.max(0, (event.clientX - box.left - size / 2) / (box.width - size)),
    );
    hover = min + fraction * (max - min);
  }
  function left(at: number) {
    if (!input) return 0;
    const size = thumb();
    const fraction = max > min ? (at - min) / (max - min) : 0;
    const centre = size / 2 + (input.offsetWidth - size) * fraction;
    const half = previewWidth / 2;
    return (
      input.offsetLeft +
      Math.min(Math.max(centre, half), input.offsetWidth - half)
    );
  }
  // Background offsets of thumbnail `index` of a sheet `count` thumbnails across.
  function offset(index: number, count: number) {
    return count > 1 ? `${(index / (count - 1)) * 100}%` : '0%';
  }
</script>

<input
  {...rest}
  bind:this={input}
  type="range"
  value={scrub ?? position}
  oninput={(event) => (scrub = Number(event.currentTarget.value))}
  onchange={(event) => {
    scrub = null;
    onseek(Number(event.currentTarget.value));
  }}
  onpointermove={hovered}
  onpointerleave={() => (hover = null)}
  onpointerup={() => setTimeout(() => (scrub = null))}
  onpointercancel={() => (scrub = null)}
/>
{#if preview && shown !== null}
  {@const index = Math.max(
    0,
    Math.min(preview.count - 1, Math.floor(shown / preview.interval)),
  )}
  {@const sheet = preview.columns * preview.rows}
  {@const cell = index % sheet}
  <div
    class="seek-preview pointer-events-none absolute z-[3] grid w-48 -translate-x-1/2 -translate-y-full border border-line bg-surface-strong text-foreground shadow-card max-[600px]:w-36 player-small:w-32"
    style:left={`${left(shown)}px`}
    style:top={`${(input?.offsetTop ?? 0) - 8}px`}
    bind:offsetWidth={previewWidth}
  >
    <div
      class="seek-preview-image bg-black bg-no-repeat"
      style:aspect-ratio={`${preview.width} / ${preview.height}`}
      style:background-image={`url("${preview.sheets[Math.floor(index / sheet)]}")`}
      style:background-size={`${preview.columns * 100}% ${preview.rows * 100}%`}
      style:background-position={`${offset(cell % preview.columns, preview.columns)} ${offset(Math.floor(cell / preview.columns), preview.rows)}`}
    ></div>
    <span class="py-0.5 text-center text-xs tabular-nums">{time(shown)}</span>
  </div>
{/if}
