import { tick } from 'svelte';
import { nextCardColumns } from './card-grid-zoom';
import type { LayoutMotionController } from './layout-motion';
import { captureScrollAnchor, restoreScrollAnchor } from './scroll-anchor';

export function createCardGridWheelHandler({
  anchorAttribute,
  motion,
  getColumns,
  setColumns,
}: {
  anchorAttribute: string;
  motion: Pick<LayoutMotionController, 'capture' | 'play'>;
  getColumns: () => number;
  setColumns: (columns: number) => void;
}) {
  return async (event: WheelEvent) => {
    if (!event.ctrlKey || event.deltaY === 0) return;
    const root = event.currentTarget as HTMLElement;
    event.preventDefault();
    if (!root.querySelector('[data-card-grid]')) return;
    const columns = getColumns();
    const next = nextCardColumns(columns, event.deltaY);
    if (next === columns) return;
    const snapshot = motion.capture();
    const anchor = captureScrollAnchor(root, anchorAttribute);
    setColumns(next);
    await tick();
    if (!root.isConnected) return;
    restoreScrollAnchor(root, anchorAttribute, anchor);
    motion.play(snapshot);
  };
}
