// Marks pinned feed date headings. One scroll listener and one frame serve
// every heading of a feed instead of one per heading.
export function createStickyGroupHeaders() {
  const headers = new Map<HTMLElement, HTMLElement>();
  let root: HTMLElement | null = null;
  let shell: HTMLElement | null = null;
  let resize: ResizeObserver | null = null;
  let frame: number | null = null;

  function update() {
    frame = null;
    if (!root) return;
    const rootTop =
      root.getBoundingClientRect().top +
      (Number.parseFloat(getComputedStyle(root).paddingTop) || 0);
    const states = [...headers].map(([header, section]) => ({
      header,
      stuck:
        section.getBoundingClientRect().top < rootTop &&
        Math.abs(header.getBoundingClientRect().top - rootTop) <= 1,
    }));
    for (const { header, stuck } of states)
      header.toggleAttribute('data-stuck', stuck);
  }

  function schedule() {
    if (frame === null) frame = requestAnimationFrame(update);
  }

  return function trackStickyGroupHeader(node: HTMLElement) {
    const scrollRoot = node.closest<HTMLElement>('[data-feed-scroll]');
    const section = node.closest<HTMLElement>('section');
    if (!scrollRoot || !section) return;
    if (!root) {
      root = scrollRoot;
      shell = scrollRoot.closest('[data-sidebar-shell]');
      resize = new ResizeObserver(schedule);
      resize.observe(scrollRoot);
      scrollRoot.addEventListener('scroll', schedule, { passive: true });
      shell?.addEventListener('sidebar-motion-end', schedule);
    }
    headers.set(node, section);
    resize?.observe(section);
    schedule();
    return {
      destroy() {
        headers.delete(node);
        resize?.unobserve(section);
        node.removeAttribute('data-stuck');
        if (headers.size > 0) return;
        root?.removeEventListener('scroll', schedule);
        shell?.removeEventListener('sidebar-motion-end', schedule);
        resize?.disconnect();
        if (frame !== null) cancelAnimationFrame(frame);
        frame = null;
        root = null;
        shell = null;
        resize = null;
      },
    };
  };
}
