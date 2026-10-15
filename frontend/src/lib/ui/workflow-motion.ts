import { tick } from 'svelte';
import { playAnimations } from '../layout-animation';

export function animateWorkflowDialog(
  dialog: HTMLDialogElement,
  content: HTMLElement,
) {
  const opening = playAnimations([
    {
      element: dialog,
      keyframes: [
        { opacity: 0, transform: 'translateY(12px)' },
        { opacity: 1, transform: 'none' },
      ],
      options: { fill: 'both' },
    },
    {
      element: dialog,
      keyframes: [{ opacity: 0 }, { opacity: 1 }],
      options: { pseudoElement: '::backdrop', fill: 'both' },
    },
  ]);
  void Promise.allSettled(opening.map((animation) => animation.finished)).then(
    () => opening.forEach((animation) => animation.cancel()),
  );

  let previousHeight = dialog.offsetHeight;
  let resizing: Animation | undefined;
  const resize = () => {
    const from = resizing ? dialog.offsetHeight : previousHeight;
    resizing?.cancel();
    // With the previous animation removed, the browser measures the natural
    // content height and applies the dialog's responsive viewport constraints.
    const height = dialog.offsetHeight;
    previousHeight = height;
    resizing = undefined;
    if (Math.abs(from - height) < 1) return;
    const [animation] = playAnimations([
      {
        element: dialog,
        keyframes: [{ height: `${from}px` }, { height: `${height}px` }],
        options: { fill: 'both' },
      },
    ]);
    resizing = animation;
    if (animation)
      void animation.finished.then(
        () => {
          if (resizing !== animation) return;
          animation.cancel();
          resizing = undefined;
          previousHeight = dialog.offsetHeight;
        },
        () => {},
      );
  };
  const observer = new ResizeObserver(resize);
  observer.observe(content);
  window.addEventListener('resize', resize);
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  const reduce = () => {
    if (!reduced.matches) return;
    opening.forEach((animation) => animation.cancel());
    resizing?.cancel();
    resizing = undefined;
    previousHeight = dialog.offsetHeight;
  };
  reduced.addEventListener('change', reduce);
  return () => {
    observer.disconnect();
    window.removeEventListener('resize', resize);
    reduced.removeEventListener('change', reduce);
    opening.forEach((animation) => animation.cancel());
    resizing?.cancel();
  };
}

export function stepHighlight(node: HTMLElement, selected: string | undefined) {
  const root = node.parentElement!;
  let key = selected;
  let positioned = false;
  let animation: Animation | undefined;
  let generation = 0;
  const position = () => {
    if (!key) return;
    const target = root.querySelector<HTMLElement>(
      `[data-transfer-step="${CSS.escape(key)}"]`,
    );
    if (!target) return;
    const rootBounds = root.getBoundingClientRect();
    const bounds = target.getBoundingClientRect();
    const top = bounds.top - rootBounds.top + root.scrollTop;
    const from =
      node.getBoundingClientRect().top - rootBounds.top + root.scrollTop;
    const fromHeight = node.offsetHeight;
    const initial = !positioned;
    animation?.cancel();
    node.style.height = `${bounds.height}px`;
    node.style.transform = `translateY(${top}px)`;
    animation = undefined;
    if (
      positioned &&
      (Math.abs(from - top) >= 0.5 ||
        Math.abs(fromHeight - bounds.height) >= 0.5)
    ) {
      [animation] = playAnimations([
        {
          element: node,
          keyframes: [
            { transform: `translateY(${from}px)`, height: `${fromHeight}px` },
            { transform: `translateY(${top}px)`, height: `${bounds.height}px` },
          ],
          options: { fill: 'both' },
        },
      ]);
      const current = animation;
      if (current)
        void current.finished.then(
          () => {
            if (animation !== current) return;
            current.cancel();
            animation = undefined;
          },
          () => {},
        );
    }
    positioned = true;
    if (root.scrollHeight > root.clientHeight) {
      root.scrollTo({
        top: top - (root.clientHeight - bounds.height) / 2,
        behavior: initial || reduced.matches ? 'instant' : 'smooth',
      });
    }
  };
  const update = (next: string | undefined) => {
    key = next;
    const current = ++generation;
    void tick().then(() => {
      if (generation === current) position();
    });
  };
  const observer = new ResizeObserver(position);
  observer.observe(root);
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
  reduced.addEventListener('change', position);
  update(selected);
  return {
    update,
    destroy() {
      generation++;
      observer.disconnect();
      reduced.removeEventListener('change', position);
      animation?.cancel();
    },
  };
}
