export interface ScrollAnchor {
  id: string;
  offset: number;
}

export function captureScrollAnchor(
  root: HTMLElement | null,
  attribute: string,
): ScrollAnchor | null {
  if (!root) return null;
  const rootTop = root.getBoundingClientRect().top;
  const item = [...root.querySelectorAll<HTMLElement>(`[${attribute}]`)].find(
    (item) => item.getBoundingClientRect().bottom > rootTop,
  );
  if (!item) return null;
  return {
    id: item.getAttribute(attribute) ?? '',
    offset: item.getBoundingClientRect().top - rootTop,
  };
}

export function restoreScrollAnchor(
  root: HTMLElement | null,
  attribute: string,
  anchor: ScrollAnchor | null,
) {
  if (!root || !anchor) return;
  const item = [...root.querySelectorAll<HTMLElement>(`[${attribute}]`)].find(
    (item) => item.getAttribute(attribute) === anchor.id,
  );
  if (item) {
    root.scrollTop +=
      item.getBoundingClientRect().top -
      root.getBoundingClientRect().top -
      anchor.offset;
  }
}
