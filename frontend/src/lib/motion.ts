import { readonly, writable } from 'svelte/store';

export const defaultAnimationDuration = 200;
let duration = 0;
const preference = writable(duration);
export const animationDuration = readonly(preference);
let reduced: MediaQueryList | undefined;
const scrolling = new Map<HTMLElement, (finish?: boolean) => void>();

export function getAnimationDuration(): number {
  return (reduced ?? window.matchMedia('(prefers-reduced-motion: reduce)'))
    .matches
    ? 0
    : duration;
}

export function animationsDisabled(): boolean {
  return getAnimationDuration() === 0;
}

function applyMotion() {
  const effective = getAnimationDuration();
  document.documentElement.style.setProperty(
    '--animation-duration',
    `${effective}ms`,
  );
  document.documentElement.dataset.motion = effective ? 'on' : 'off';
  // Finish finite effects so their owners still run completion/cleanup logic.
  // This includes Svelte transitions and FLIP, as well as our Web Animations.
  for (const animation of document.getAnimations()) {
    if (
      animation instanceof CSSTransition &&
      animation.effect instanceof KeyframeEffect &&
      animation.effect.target?.hasAttribute('data-motion-indicator')
    )
      continue;
    if (animation.playState === 'finished' || animation.playState === 'idle')
      continue;
    const end = animation.effect?.getComputedTiming().endTime;
    if (typeof end === 'number' && Number.isFinite(end)) animation.finish();
  }
  for (const finish of scrolling.values()) finish(true);
}

export function setAnimationDuration(value: number) {
  if (!reduced) {
    reduced = window.matchMedia('(prefers-reduced-motion: reduce)');
    reduced.addEventListener('change', applyMotion);
  }
  const next = Number.isFinite(value)
    ? Math.max(0, Math.min(500, Math.round(value)))
    : defaultAnimationDuration;
  if (duration === next && document.documentElement.dataset.motion) return;
  duration = next;
  preference.set(next);
  applyMotion();
}

// Native smooth scrolling has a browser-chosen duration. Keep programmatic
// scrolling on the same clock, while wheel/touch/pointer input stays direct.
export function scrollWithMotion(
  element: HTMLElement,
  target: { top?: number; left?: number },
  animate = true,
) {
  scrolling.get(element)?.();
  const from = { top: element.scrollTop, left: element.scrollLeft };
  const to = {
    top: Math.max(
      0,
      Math.min(
        target.top ?? from.top,
        element.scrollHeight - element.clientHeight,
      ),
    ),
    left: Math.max(
      0,
      Math.min(
        target.left ?? from.left,
        element.scrollWidth - element.clientWidth,
      ),
    ),
  };
  const duration = animate ? getAnimationDuration() : 0;
  let lastTop = from.top;
  let lastLeft = from.left;
  const scroll = (progress: number) => {
    element.scrollTo({
      top: from.top + (to.top - from.top) * progress,
      left: from.left + (to.left - from.left) * progress,
      behavior: 'instant',
    });
    lastTop = element.scrollTop;
    lastLeft = element.scrollLeft;
  };
  if (!duration || (from.top === to.top && from.left === to.left)) {
    scroll(1);
    return;
  }
  let frame = 0;
  const controller = new AbortController();
  const stop = (finish = false) => {
    cancelAnimationFrame(frame);
    controller.abort();
    scrolling.delete(element);
    if (finish) scroll(1);
  };
  scrolling.set(element, stop);
  for (const event of ['wheel', 'touchstart', 'pointerdown', 'keydown'])
    element.addEventListener(event, () => stop(), {
      passive: true,
      signal: controller.signal,
    });
  const start = performance.now();
  const step = (now: number) => {
    if (
      !element.isConnected ||
      Math.abs(element.scrollTop - lastTop) > 1 ||
      Math.abs(element.scrollLeft - lastLeft) > 1
    ) {
      stop();
      return;
    }
    const progress = Math.min(1, (now - start) / duration);
    scroll(1 - (1 - progress) ** 3);
    if (progress < 1) frame = requestAnimationFrame(step);
    else stop();
  };
  frame = requestAnimationFrame(step);
}
