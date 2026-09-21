import {
  cancelLayoutAnimations,
  feedTransformPlans,
  hasTransform,
  mediaZoom,
  playAnimations,
  prefersReducedMotion,
  rectTransform,
  transformKeyframes,
  type AnimationPlan,
  type LayoutRect,
  type LayoutTransform,
  type PositionSnapshot,
} from './layout-animation';

interface SidebarOrigin extends PositionSnapshot {
  mode: string;
}

interface SidebarSnapshot extends SidebarOrigin {
  scrollTop: number;
  origin?: SidebarOrigin;
}

export function captureSidebarResize(
  root: HTMLElement | null,
  enabled = true,
): SidebarSnapshot[] {
  if (!root) return [];
  const elements = [
    ...root.querySelectorAll<HTMLElement>('[data-sidebar-resize]'),
  ];
  const animatedElements = new Set(elements);
  const origins = new Map<HTMLElement, SidebarOrigin>();
  const snapshots = enabled
    ? elements.map((element) => {
        const candidate = element.parentElement?.closest<HTMLElement>(
          '[data-sidebar-resize-origin]',
        );
        // Local grid/watchlist zooms must not counter a surrounding viewport
        // that is outside this animation and is not moving with them.
        const originElement =
          candidate && animatedElements.has(candidate) ? candidate : undefined;
        if (originElement && !origins.has(originElement)) {
          origins.set(originElement, {
            element: originElement,
            rect: originElement.getBoundingClientRect(),
            mode: originElement.dataset.sidebarResize ?? 'xy',
          });
        }
        return {
          element,
          mode: element.dataset.sidebarResize ?? 'xy',
          rect: element.getBoundingClientRect(),
          scrollTop: element.parentElement?.scrollTop ?? 0,
          origin: originElement ? origins.get(originElement) : undefined,
        };
      })
    : [];
  cancelLayoutAnimations(elements);
  return snapshots;
}

export function sidebarResizeTransform(
  previous: LayoutRect,
  final: LayoutRect,
  mode: string,
  zoom: number,
  previousScrollTop: number,
  finalScrollTop: number,
): LayoutTransform {
  const transform = rectTransform(previous, final, zoom);
  if (!['x', 'xy', 'x-pos', 'xy-pos'].includes(mode)) transform.x = 0;
  if (!['y', 'xy', 'y-pos', 'xy-pos'].includes(mode)) transform.y = 0;
  if (!['x', 'xy'].includes(mode)) transform.scaleX = 1;
  if (!['y', 'xy', 'y-scale', 'y-scale-scroll'].includes(mode))
    transform.scaleY = 1;
  if (mode === 'y-scale-scroll') {
    transform.y += finalScrollTop - transform.scaleY * previousScrollTop;
  }
  return transform;
}

function originKeyframes(
  transform: LayoutTransform,
  final: LayoutRect,
  origin: LayoutRect,
  originTransform: LayoutTransform,
  zoom: number,
  mode: string,
): Keyframe[] {
  const left = final.left - origin.left;
  const top = final.top - origin.top;
  // Compensate for the moving scroll viewport throughout the animation.
  // Endpoint-only inverse scales make fixed-size controls swell in between.
  return Array.from({ length: 9 }, (_, index) => {
    const offset = index / 8;
    const remaining = 1 - offset;
    const scaleX = 1 + (originTransform.scaleX - 1) * remaining;
    const scaleY = 1 + (originTransform.scaleY - 1) * remaining;
    const relative: LayoutTransform = {
      x:
        ((left + (transform.x * zoom - originTransform.x) * remaining) /
          scaleX -
          left) /
        zoom,
      y:
        ((top + (transform.y * zoom - originTransform.y) * remaining) / scaleY -
          top) /
        zoom,
      scaleX: (1 + (transform.scaleX - 1) * remaining) / scaleX,
      scaleY: (1 + (transform.scaleY - 1) * remaining) / scaleY,
    };
    // A one-axis child keeps the other axis supplied by its moving viewport.
    // Countering both axes would pin vertical-only feed rows to their final X.
    if (!['x', 'xy', 'x-pos', 'xy-pos', 'video'].includes(mode)) {
      relative.x = 0;
      relative.scaleX = 1;
    }
    if (
      ![
        'y',
        'xy',
        'y-pos',
        'xy-pos',
        'y-scale',
        'y-scale-scroll',
        'video',
      ].includes(mode)
    ) {
      relative.y = 0;
      relative.scaleY = 1;
    }
    return { ...transformKeyframes(relative)[0], offset };
  });
}

export function sidebarResizePlans(
  snapshots: SidebarSnapshot[],
): AnimationPlan[] {
  if (prefersReducedMotion()) return [];
  const zooms = new Map<Element, number>();
  const origins = new Map<HTMLElement, LayoutRect>();
  const plans: AnimationPlan[] = [];
  for (const { element, rect, mode, scrollTop, origin } of snapshots) {
    const final = element.getBoundingClientRect();
    if (final.width <= 0 || final.height <= 0) continue;
    // Resize a scrolling navigation viewport in layout: its buttons and rail
    // then stay inside the moving clip without scaling icons or labels.
    if (mode === 'width') {
      if (Math.abs(rect.width - final.width) >= 0.5)
        plans.push({
          element,
          keyframes: [
            { width: `${rect.width}px` },
            { width: `${final.width}px` },
          ],
        });
      continue;
    }
    let zoom = 1;
    if (element.hasAttribute('data-sidebar-resize-zoom')) {
      const owner =
        element.closest('[data-youtube-watchlist-scale]') ?? element;
      if (!zooms.has(owner)) zooms.set(owner, mediaZoom(owner));
      zoom = zooms.get(owner)!;
    }
    const transform = sidebarResizeTransform(
      rect,
      final,
      mode,
      zoom,
      scrollTop,
      element.parentElement?.scrollTop ?? 0,
    );
    if (mode === 'video') {
      const video = element as HTMLVideoElement;
      const aspect =
        video.videoWidth / video.videoHeight || final.width / final.height;
      const scale =
        Math.min(rect.width / aspect, rect.height) /
        Math.min(final.width / aspect, final.height);
      // Keep the decoded picture proportional and centered while letterboxing
      // changes. Scaling the player itself would otherwise stretch the video.
      transform.x =
        rect.left +
        rect.width / 2 -
        (final.left + final.width / 2) +
        (final.width * (1 - scale)) / 2;
      transform.y =
        rect.top +
        rect.height / 2 -
        (final.top + final.height / 2) +
        (final.height * (1 - scale)) / 2;
      transform.scaleX = scale;
      transform.scaleY = scale;
    }
    if (origin) {
      if (!origins.has(origin.element)) {
        origins.set(origin.element, origin.element.getBoundingClientRect());
      }
      const finalOrigin = origins.get(origin.element)!;
      const originTransform = sidebarResizeTransform(
        origin.rect,
        finalOrigin,
        origin.mode,
        1,
        0,
        0,
      );
      if (!hasTransform(transform) && !hasTransform(originTransform)) continue;
      plans.push({
        element,
        keyframes: originKeyframes(
          transform,
          final,
          finalOrigin,
          originTransform,
          zoom,
          mode,
        ),
      });
    } else if (!hasTransform(transform)) continue;
    else if (element.hasAttribute('data-feed-scroll')) {
      plans.push(...feedTransformPlans(element, transform, final));
    } else {
      plans.push({ element, keyframes: transformKeyframes(transform) });
    }
  }
  return plans;
}

export function createSidebarMotion() {
  let generation = 0;
  let animations: Animation[] = [];
  let clippedMain: HTMLElement | null = null;

  function restoreClip() {
    clippedMain?.style.removeProperty('overflow');
    clippedMain = null;
  }

  return {
    capture(root: HTMLElement | null, main: HTMLElement | null) {
      const current = ++generation;
      restoreClip();
      clippedMain = main;
      if (main) main.style.overflow = 'visible';
      const snapshots = captureSidebarResize(root, !prefersReducedMotion());
      for (const animation of animations) animation.cancel();
      animations = [];
      return () => {
        if (current !== generation) return;
        animations = playAnimations(sidebarResizePlans(snapshots));
        if (animations.length === 0) {
          restoreClip();
          return;
        }
        void Promise.allSettled(
          animations.map((animation) => animation.finished),
        ).then(() => {
          if (current !== generation) return;
          animations = [];
          restoreClip();
        });
      };
    },
    destroy() {
      generation += 1;
      for (const animation of animations) animation.cancel();
      animations = [];
      restoreClip();
    },
  };
}
