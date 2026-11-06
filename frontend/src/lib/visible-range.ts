/** How many rows past the viewport render, so scrolling doesn't reveal blanks. */
const OVERSCAN = 6;

/** Which rows of a list of equal rows are in or near the viewport. */
export function visibleRange({
  count,
  rowHeight,
  scrollTop,
  viewportHeight,
}: {
  count: number;
  rowHeight: number;
  scrollTop: number;
  viewportHeight: number;
}) {
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + viewportHeight) / rowHeight);
  const start = Math.min(Math.max(0, first - OVERSCAN), Math.max(0, count - 1));
  return { start, end: Math.min(count, last + OVERSCAN) };
}
