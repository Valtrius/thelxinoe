export const layoutAnimationTiming = {
  duration: 200,
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
}

export function prefersReducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
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
    for (const animation of element.getAnimations()) animation.cancel();
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
  if (prefersReducedMotion()) return [];
  return plans.map(({ element, keyframes, options }) =>
    element.animate(keyframes, { ...timing, ...options }),
  );
}
