import {
  isVisible,
  layoutAnimationTiming,
  mediaZoom,
  playAnimations,
  prefersReducedMotion,
  rectTransform,
  transformKeyframes,
  type AnimationPlan,
  type LayoutRect,
  type LayoutTransform,
} from './layout-animation';

export type { LayoutRect } from './layout-animation';

export interface LayoutMotionOptions {
  durationMs: number;
  easing: string;
  itemScale: number;
  animateWidth: boolean;
  fadeEntries: boolean;
  fadeRemovals: boolean;
  animateInitialItems: boolean;
  moveTolerancePixels: number;
  sizeTolerancePixels: number;
  visibleMarginPixels: number;
}

export interface LayoutItemSnapshot {
  element: HTMLElement;
  rect: LayoutRect;
  cardGridZoom?: number;
}

export interface LayoutSnapshot {
  items: Map<string, LayoutItemSnapshot>;
  rootRect: LayoutRect;
}

export const feedLayoutMotionOptions: Readonly<LayoutMotionOptions> = {
  durationMs: layoutAnimationTiming.duration,
  easing: layoutAnimationTiming.easing,
  itemScale: 0.9,
  animateWidth: true,
  fadeEntries: true,
  fadeRemovals: true,
  animateInitialItems: false,
  moveTolerancePixels: 0.5,
  sizeTolerancePixels: 0.75,
  visibleMarginPixels: 240,
};

const itemSelector = '[data-layout-key]';
const connectedLayoutMotions = new Set<LayoutMotionController>();

function itemKey(element: HTMLElement): string | null {
  return element.dataset.layoutKey?.trim() || null;
}

function gridZoom(
  element: HTMLElement,
  zooms: Map<HTMLElement, number>,
): number | undefined {
  const parent = element.parentElement;
  if (!parent?.hasAttribute('data-card-grid')) return undefined;
  if (!zooms.has(parent)) {
    zooms.set(parent, Number.parseFloat(getComputedStyle(parent).zoom) || 1);
  }
  return zooms.get(parent);
}

export function entryKeyframes(scale: number, fade: boolean): Keyframe[] {
  if (!fade)
    return [{ transform: `scale(${scale})` }, { transform: 'scale(1)' }];
  return [
    { opacity: 0, transform: `scale(${scale})` },
    { opacity: 1, transform: 'scale(1)' },
  ];
}

export function removalKeyframes(scale: number, fade: boolean): Keyframe[] {
  if (!fade)
    return [{ transform: 'scale(1)' }, { transform: `scale(${scale})` }];
  return [
    { opacity: 1, transform: 'scale(1)' },
    { opacity: 0, transform: `scale(${scale})` },
  ];
}

export function layoutTransform(
  previous: LayoutRect,
  final: LayoutRect,
  options: Pick<
    LayoutMotionOptions,
    'animateWidth' | 'moveTolerancePixels' | 'sizeTolerancePixels'
  >,
  includeHeight = false,
): LayoutTransform | null {
  const transform = rectTransform(previous, final);
  if (!options.animateWidth) transform.scaleX = 1;
  if (!includeHeight) transform.scaleY = 1;
  if (
    Math.abs(transform.x) <= options.moveTolerancePixels &&
    Math.abs(transform.y) <= options.moveTolerancePixels &&
    Math.abs(previous.width - final.width) <= options.sizeTolerancePixels &&
    (!includeHeight ||
      Math.abs(previous.height - final.height) <= options.sizeTolerancePixels)
  )
    return null;
  return transform;
}

export class LayoutMotionController {
  private root: HTMLElement | null = null;
  private readonly options: Readonly<LayoutMotionOptions>;
  private activeAnimations = new Set<Animation>();
  private ghosts = new Set<HTMLElement>();
  private generation = 0;
  private hasPresentedItems = false;

  constructor(options: Partial<LayoutMotionOptions> = {}) {
    this.options = { ...feedLayoutMotionOptions, ...options };
  }

  readonly connect = (root: HTMLElement) => {
    this.disconnect();
    this.root = root;
    connectedLayoutMotions.add(this);
    this.hasPresentedItems = this.currentItems().length > 0;
    return { destroy: () => this.disconnect() };
  };

  capture(): LayoutSnapshot | null {
    const snapshot = this.captureCurrent();
    this.stopAnimations();
    return snapshot;
  }

  play(snapshot: LayoutSnapshot | null): void {
    const root = this.root;
    if (!root || !snapshot) return;
    this.stopAnimations();
    const elements = this.currentItems();
    const rootRect = root.getBoundingClientRect();
    const generation = ++this.generation;
    if (!prefersReducedMotion()) {
      const rootZoom = mediaZoom(root);
      const zooms = new Map<HTMLElement, number>();
      const currentKeys = new Set<string>();
      const plans: AnimationPlan[] = [];
      const animateEntries =
        this.options.animateInitialItems || this.hasPresentedItems;
      // Finish all geometry/style reads before creating any animations or ghosts.
      for (const element of elements) {
        const key = itemKey(element);
        if (!key || currentKeys.has(key)) continue;
        currentKeys.add(key);
        const final = element.getBoundingClientRect();
        if (!isVisible(final, rootRect, this.options.visibleMarginPixels))
          continue;
        const previous = snapshot.items.get(key);
        if (!previous) {
          if (animateEntries)
            plans.push({
              element,
              keyframes: entryKeyframes(
                this.options.itemScale,
                this.options.fadeEntries,
              ),
              options: { fill: 'backwards' },
            });
          continue;
        }
        const cardGridZoom = gridZoom(element, zooms);
        const transform = layoutTransform(
          previous.rect,
          final,
          this.options,
          cardGridZoom !== undefined,
        );
        if (!transform) continue;
        const zoom = cardGridZoom ?? rootZoom;
        transform.x /= zoom;
        transform.y /= zoom;
        plans.push({ element, keyframes: transformKeyframes(transform) });
      }
      plans.push(
        ...this.removalPlans(snapshot, currentKeys, rootRect, rootZoom),
      );
      for (const animation of playAnimations(plans, {
        duration: this.options.durationMs,
        easing: this.options.easing,
      }))
        this.activeAnimations.add(animation);
    }
    if (elements.length > 0) this.hasPresentedItems = true;
    if (this.activeAnimations.size === 0) return;
    void Promise.allSettled(
      [...this.activeAnimations].map((animation) => animation.finished),
    ).then(() => {
      if (generation !== this.generation) return;
      for (const ghost of this.ghosts) ghost.remove();
      this.ghosts.clear();
      this.activeAnimations.clear();
    });
  }

  disconnect(): void {
    connectedLayoutMotions.delete(this);
    this.stopAnimations();
    this.root = null;
    this.hasPresentedItems = false;
  }

  settle(): void {
    this.stopAnimations();
  }

  private currentItems(): HTMLElement[] {
    return this.root
      ? [...this.root.querySelectorAll<HTMLElement>(itemSelector)]
      : [];
  }

  private captureCurrent(): LayoutSnapshot | null {
    if (!this.root) return null;
    const items = new Map<string, LayoutItemSnapshot>();
    const zooms = new Map<HTMLElement, number>();
    for (const element of this.currentItems()) {
      const key = itemKey(element);
      if (!key || items.has(key)) continue;
      items.set(key, {
        element,
        rect: element.getBoundingClientRect(),
        cardGridZoom: gridZoom(element, zooms),
      });
    }
    return { items, rootRect: this.root.getBoundingClientRect() };
  }

  private removalPlans(
    snapshot: LayoutSnapshot,
    currentKeys: Set<string>,
    rootRect: LayoutRect,
    rootZoom: number,
  ): AnimationPlan[] {
    const root = this.root;
    if (!root) return [];
    const plans: AnimationPlan[] = [];
    const scrollTop = root.scrollTop;
    const scrollLeft = root.scrollLeft;
    for (const [key, previous] of snapshot.items) {
      if (
        currentKeys.has(key) ||
        !isVisible(
          previous.rect,
          snapshot.rootRect,
          this.options.visibleMarginPixels,
        )
      )
        continue;
      const clone = previous.element.cloneNode(true) as HTMLElement;
      clone.removeAttribute('data-layout-key');
      clone.removeAttribute('data-watchlist-video-id');
      // Ghosts are presentation only, including removed group headings.
      clone.removeAttribute('data-card-group-header');
      clone.removeAttribute('data-sidebar-resize');
      for (const anchor of clone.querySelectorAll<HTMLElement>(
        '[data-video-id], [data-stream-id], [data-channel-slug]',
      )) {
        anchor.removeAttribute('data-video-id');
        anchor.removeAttribute('data-stream-id');
        anchor.removeAttribute('data-channel-slug');
      }
      const zoom = previous.cardGridZoom ?? 1;
      const ghost = zoom === 1 ? clone : document.createElement('div');
      if (zoom !== 1) {
        Object.assign(clone.style, {
          width: `${previous.rect.width / zoom}px`,
          height: `${previous.rect.height / zoom}px`,
          margin: '0',
          zoom: String(zoom),
        });
        ghost.appendChild(clone);
      }
      ghost.setAttribute('aria-hidden', 'true');
      ghost.inert = true;
      Object.assign(ghost.style, {
        position: 'absolute',
        zIndex: '100',
        top: `${(previous.rect.top - rootRect.top) / rootZoom + scrollTop}px`,
        left: `${(previous.rect.left - rootRect.left) / rootZoom + scrollLeft}px`,
        width: `${previous.rect.width / rootZoom}px`,
        height: `${previous.rect.height / rootZoom}px`,
        margin: '0',
        pointerEvents: 'none',
        transformOrigin: 'center',
        willChange: 'transform',
      });
      root.appendChild(ghost);
      this.ghosts.add(ghost);
      plans.push({
        element: ghost,
        keyframes: removalKeyframes(
          this.options.itemScale,
          this.options.fadeRemovals,
        ),
        options: { fill: 'forwards' },
      });
    }
    return plans;
  }

  private stopAnimations(): void {
    this.generation += 1;
    for (const animation of this.activeAnimations) animation.cancel();
    this.activeAnimations.clear();
    for (const ghost of this.ghosts) ghost.remove();
    this.ghosts.clear();
  }
}

export function createLayoutMotion(
  options: Partial<LayoutMotionOptions> = {},
): LayoutMotionController {
  return new LayoutMotionController(options);
}

export function settleLayoutMotions(): void {
  for (const motion of connectedLayoutMotions) motion.settle();
}
