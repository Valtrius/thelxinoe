import { beforeEach, describe, expect, it, vi } from 'vitest';
import { tick } from 'svelte';
import { createCardGridWheelHandler } from './card-grid-wheel';
import { captureScrollAnchor, restoreScrollAnchor } from './scroll-anchor';

vi.mock('svelte', () => ({ tick: vi.fn() }));
vi.mock('./scroll-anchor', () => ({
  captureScrollAnchor: vi.fn(() => ({ id: 'visible-video', offset: -12 })),
  restoreScrollAnchor: vi.fn(),
}));

describe('shared card wheel transaction', () => {
  beforeEach(() => vi.clearAllMocks());

  function fixture(columns = 6) {
    const viewport = {};
    const root = {
      querySelector: () => ({}),
      closest: () => viewport,
      isConnected: true,
    };
    const motion = { capture: vi.fn(() => null), play: vi.fn() };
    const setColumns = vi.fn((next: number) => {
      columns = next;
    });
    const handle = createCardGridWheelHandler({
      anchorAttribute: 'data-video-id',
      motion,
      getColumns: () => columns,
      setColumns,
    });
    const event = {
      ctrlKey: true,
      deltaY: 100,
      currentTarget: root,
      preventDefault: vi.fn(),
    };
    return {
      root,
      viewport,
      motion,
      setColumns,
      event,
      handle: () => handle(event as unknown as WheelEvent),
    };
  }

  it('restores the same scroll anchor only after the new layout is ready', async () => {
    let finishTick!: () => void;
    vi.mocked(tick).mockReturnValue(
      new Promise<void>((resolve) => {
        finishTick = resolve;
      }),
    );
    const { viewport, motion, setColumns, handle } = fixture();
    const pending = handle();
    expect(setColumns).toHaveBeenCalledWith(7);
    expect(captureScrollAnchor).toHaveBeenCalledWith(viewport, 'data-video-id');
    expect(restoreScrollAnchor).not.toHaveBeenCalled();
    expect(motion.play).not.toHaveBeenCalled();
    finishTick();
    await pending;
    expect(restoreScrollAnchor).toHaveBeenCalledWith(
      viewport,
      'data-video-id',
      {
        id: 'visible-video',
        offset: -12,
      },
    );
    expect(motion.play).toHaveBeenCalledOnce();
  });

  it('leaves ordinary scrolling alone and avoids work at the column limit', async () => {
    const { event, motion, handle, setColumns } = fixture(12);
    event.ctrlKey = false;
    await handle();
    expect(event.preventDefault).not.toHaveBeenCalled();
    event.ctrlKey = true;
    await handle();
    expect(event.preventDefault).toHaveBeenCalledOnce();
    expect(setColumns).not.toHaveBeenCalled();
    expect(motion.capture).not.toHaveBeenCalled();
  });

  it('does not replay an animation after the view is removed during the DOM update', async () => {
    const { root, motion, handle } = fixture();
    vi.mocked(tick).mockImplementation(async () => {
      root.isConnected = false;
    });
    await handle();
    expect(restoreScrollAnchor).not.toHaveBeenCalled();
    expect(motion.play).not.toHaveBeenCalled();
  });
});
