<script lang="ts">
  import { X } from '@lucide/svelte';
  import { time, type MediaChoice } from '../playback';
  import Button from '../ui/Button.svelte';
  import { visibleRange } from '../visible-range';
  import PlayingBars from './PlayingBars.svelte';

  const ROW_HEIGHT = 48;

  let {
    queue,
    from,
    index,
    paused,
    jump,
    remove,
  }: {
    queue: MediaChoice[];
    /** The first place of the queue to list. */
    from: number;
    index: number;
    paused: boolean;
    jump: (at: number) => void;
    remove: (at: number) => void;
  } = $props();

  const count = $derived(Math.max(0, queue.length - from));
  let scrollTop = $state(0);
  let viewportHeight = $state(0);
  const range = $derived(
    visibleRange({ count, rowHeight: ROW_HEIGHT, scrollTop, viewportHeight }),
  );
  const rows = $derived(
    queue
      .slice(from + range.start, from + range.end)
      .map((entry: MediaChoice, offset: number) => ({
        entry,
        at: from + range.start + offset,
      })),
  );

  // The list scrolls with whatever surrounds it.
  function followScroll(node: HTMLElement) {
    const scroller = node.closest<HTMLElement>('[data-queue-scroller]')!;
    const update = () => {
      const offset =
        node.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      scrollTop = -offset;
      viewportHeight = scroller.clientHeight;
    };
    const resize = new ResizeObserver(update);
    resize.observe(scroller);
    scroller.addEventListener('scroll', update, { passive: true });
    update();
    return {
      destroy() {
        resize.disconnect();
        scroller.removeEventListener('scroll', update);
      },
    };
  }
</script>

<ol
  use:followScroll
  class="m-0 list-none p-0"
  style:height={`${count * ROW_HEIGHT}px`}
  style:padding-top={`${range.start * ROW_HEIGHT}px`}
>
  {#each rows as { entry, at } (`${at}:${entry.id}`)}
    {@const playing = at === index}
    <li
      class={[
        'group/queue flex h-12 items-center gap-1 border-t border-line pr-1.5 first:border-t-0',
        playing && 'bg-accent-soft text-accent',
      ]}
    >
      <button
        type="button"
        class="flex min-w-0 flex-1 items-center gap-2.5 bg-transparent py-1.5 pl-3 text-left text-[12.5px] text-inherit"
        aria-label={`Play ${entry.title}`}
        aria-current={playing ? 'true' : undefined}
        onclick={() => jump(at)}
      >
        <span class="flex w-5 shrink-0 justify-end text-[11px] text-muted"
          >{#if playing}<PlayingBars {paused} />{:else}{at + 1}{/if}</span
        >
        <span class="min-w-0 flex-1 truncate"
          >{entry.title}{#if entry.artist}<span
              class="block truncate text-[11px] text-muted">{entry.artist}</span
            >{/if}</span
        >
        {#if entry.duration}<span
            class="shrink-0 text-[11px] text-muted tabular-nums"
            >{time(entry.duration)}</span
          >{/if}
      </button>
      {#if !playing}<Button
          variant="player"
          size="compact-icon"
          class="text-muted opacity-0 group-hover/queue:opacity-100 focus-visible:opacity-100 compact:opacity-100"
          aria-label={`Remove ${entry.title} from the queue`}
          title="Remove from the queue"
          onclick={() => remove(at)}><X size={14} /></Button
        >{/if}
    </li>
  {/each}
</ol>
