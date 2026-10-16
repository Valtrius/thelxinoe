import { getAnimationDuration } from './motion';
export { animationsDisabled } from './motion';

export const layoutAnimationTiming = {
  get duration() {
    return getAnimationDuration();
  },
  easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
} as const;

export interface LayoutRect {
  top: number;
  right: number;
  bottom: number;
  left: number;
  width: number;
  height: number;
}

export interface LayoutTransform {
  x: number;
  y: number;
  scaleX: number;
  scaleY: number;
}

export interface PositionSnapshot {
  element: HTMLElement;
  rect: LayoutRect;
}

export interface AnimationPlan {
  element: HTMLElement;
  keyframes: Keyframe[];
  options?: KeyframeAnimationOptions;
  // Cancelling the owner also cancels this animation. Defaults to element.
  owner?: HTMLElement;
}

// Track our own animations: Element.getAnimations() flushes style on every
// call, which turns cancelling a long list of elements into repeated recalcs.
const layoutAnimations = new WeakMap<Element, Set<Animation>>();

function trackAnimation(
  owner: Element,
  animation: Animation,
  fill: FillMode | undefined,
) {
  let owned = layoutAnimations.get(owner);
  if (!owned) layoutAnimations.set(owner, (owned = new Set()));
  owned.add(animation);
  const release = () => owned.delete(animation);
  animation.addEventListener('cancel', release);
  animation.addEventListener('remove', release);
  // Filling animations stay in effect after finishing and remain cancellable.
  if (fill !== 'forwards' && fill !== 'both')
    animation.addEventListener('finish', release);
}

export function rectTransform(
  previous: LayoutRect,
  final: LayoutRect,
  zoom = 1,
): LayoutTransform {
  return {
    x: (previous.left - final.left) / zoom,
    y: (previous.top - final.top) / zoom,
    scaleX: final.width > 0 ? previous.width / final.width : 1,
    scaleY: final.height > 0 ? previous.height / final.height : 1,
  };
}

export function hasTransform(transform: LayoutTransform): boolean {
  return (
    Math.abs(transform.x) >= 0.5 ||
    Math.abs(transform.y) >= 0.5 ||
    Math.abs(transform.scaleX - 1) >= 0.001 ||
    Math.abs(transform.scaleY - 1) >= 0.001
  );
}

export function transformKeyframes(
  { x, y, scaleX, scaleY }: LayoutTransform,
  end = 'translate(0, 0) scale(1, 1)',
): Keyframe[] {
  return [
    {
      transform: `translate(${x}px, ${y}px) scale(${scaleX}, ${scaleY})`,
      transformOrigin: 'top left',
    },
    { transform: end, transformOrigin: 'top left' },
  ];
}

export function isVisible(
  rect: LayoutRect,
  root: LayoutRect,
  margin = 0,
): boolean {
  return (
    rect.bottom >= root.top - margin &&
    rect.top <= root.bottom + margin &&
    rect.right >= root.left - margin &&
    rect.left <= root.right + margin
  );
}

export function mediaZoom(element: Element): number {
  const value = Number.parseFloat(
    getComputedStyle(element).getPropertyValue('--youtube-media-scale'),
  );
  return Number.isFinite(value) && value > 0 ? value : 1;
}

export function cancelLayoutAnimations(elements: Iterable<HTMLElement>) {
  for (const element of elements) {
    const owned = layoutAnimations.get(element);
    if (owned) {
      layoutAnimations.delete(element);
      for (const animation of owned) animation.cancel();
    }
    if (element.hasAttribute('data-feed-scroll')) {
      cancelLayoutAnimations(
        element.querySelectorAll<HTMLElement>('.feed-group-label'),
      );
    }
  }
}

export function capturePositions(elements: HTMLElement[]): PositionSnapshot[] {
  return elements.map((element) => ({
    element,
    rect: element.getBoundingClientRect(),
  }));
}

export function feedTransformPlans(
  element: HTMLElement,
  transform: LayoutTransform,
  viewport: LayoutRect,
): AnimationPlan[] {
  const plans: AnimationPlan[] = [
    { element, keyframes: transformKeyframes(transform) },
  ];
  if (Math.abs(transform.scaleX - 1) < 0.001) return plans;
  for (const label of element.querySelectorAll<HTMLElement>(
    '.feed-group-label',
  )) {
    if (!isVisible(label.getBoundingClientRect(), viewport)) continue;
    // Only labels receive this non-inherited property. The feed itself can
    // animate on the compositor without restyling every loaded card.
    plans.push({
      element: label,
      keyframes: [
        { '--feed-label-scale-x': transform.scaleX },
        { '--feed-label-scale-x': 1 },
      ],
    });
  }
  return plans;
}

export function playAnimations(
  plans: AnimationPlan[],
  timing: KeyframeAnimationOptions = layoutAnimationTiming,
): Animation[] {
  const duration = getAnimationDuration();
  if (!duration) return [];
  const groupTiming = { ...timing, duration };
  const animations = plans.map(({ element, keyframes, options, owner }) => {
    const effectTiming = {
      ...groupTiming,
      ...options,
      duration,
    };
    const animation = element.animate(keyframes, effectTiming);
    trackAnimation(owner ?? element, animation, effectTiming.fill);
    return animation;
  });
  // Every group uses the same timeline origin, including long settings pages.
  const start = document.timeline.currentTime;
  if (start !== null)
    for (const animation of animations) {
      if (animation.playState !== 'paused') animation.startTime = start;
    }
  return animations;
}
