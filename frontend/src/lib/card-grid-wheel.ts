import { tick } from 'svelte';
import { nextCardColumns, type CardKind } from './card-grid-zoom';
import type { LayoutMotionController } from './layout-motion';
import { captureScrollAnchor, restoreScrollAnchor } from './scroll-anchor';
import { compactViewport } from './responsive';

export function createCardGridWheelHandler({
  kind,
  anchorAttribute,
  motion,
  getColumns,
  setColumns,
}: {
  kind: CardKind;
  anchorAttribute: string;
  motion: Pick<LayoutMotionController, 'capture' | 'play'>;
  getColumns: () => number;
  setColumns: (columns: number) => void;
}) {
  return async (event: WheelEvent) => {
    if (compactViewport.current || !event.ctrlKey || event.deltaY === 0) return;
    const root = event.currentTarget as HTMLElement;
    event.preventDefault();
    if (!root.querySelector('[data-card-grid]')) return;
    const columns = getColumns();
    const next = nextCardColumns(columns, event.deltaY, kind);
    if (next === columns) return;
    const snapshot = motion.capture();
    const viewport = root.closest<HTMLElement>('[data-feed-scroll]') ?? root;
    const anchor = captureScrollAnchor(viewport, anchorAttribute);
    setColumns(next);
    await tick();
    if (!root.isConnected) return;
    restoreScrollAnchor(viewport, anchorAttribute, anchor);
    motion.play(snapshot);
  };
}
