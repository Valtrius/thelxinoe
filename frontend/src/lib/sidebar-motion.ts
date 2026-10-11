import {
  cancelLayoutAnimations,
  hasTransform,
  isVisible,
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
  mediaScale?: number;
  mediaGap?: number;
  mediaHeight?: number;
  enter?: boolean;
  exit?: boolean;
}

type WatchlistChange = { element: HTMLElement; enter: boolean; exit: boolean };
const shellMotions = new WeakMap<
  HTMLElement,
  {
    capture: (
      root: HTMLElement,
      main: HTMLElement | null,
      watchlist?: WatchlistChange,
    ) => () => Animation[];
  }
>();

export function captureSidebarMotion(
  node: HTMLElement,
  watchlist?: WatchlistChange,
) {
  const shell = node.closest<HTMLElement>('[data-sidebar-shell]');
  return shell
    ? shellMotions.get(shell)?.capture(shell, node.closest('main'), watchlist)
    : undefined;
}

const positionX = new Set([
  'x',
  'xy',
  'x-pos',
  'xy-pos',
  'xy-width',
  'viewport',
  'video',
  'media',
]);
const positionY = new Set([
  'y',
  'xy',
  'y-pos',
  'xy-pos',
  'xy-width',
  'viewport',
  'video',
  'media',
]);

function visualRect(element: HTMLElement, mode: string): LayoutRect {
  const rect = element.getBoundingClientRect();
  if (mode !== 'viewport') return rect;
  const inset = getComputedStyle(element).clipPath.match(
    /^inset\(0px (-?[\d.]+)px 0px 0px\)$/,
  );
  if (!inset) return rect;
  const width = rect.width - Number(inset[1]);
  return {
    ...rect.toJSON(),
    left: rect.left,
    top: rect.top,
    right: rect.left + width,
    bottom: rect.bottom,
    width,
    height: rect.height,
  };
}

export function captureSidebarResize(
  root: HTMLElement | null,
  enabled = true,
): SidebarSnapshot[] {
  if (!root) return [];
  const elements = [
    ...root.querySelectorAll<HTMLElement>('[data-sidebar-resize]'),
  ].filter(
    (element) => !element.parentElement?.closest('[data-sidebar-group]'),
  );
  const animatedElements = new Set(elements);
  const origins = new Map<HTMLElement, SidebarOrigin>();
  const snapshots = enabled
    ? elements.map((element) => {
        const candidate = element.parentElement?.closest<HTMLElement>(
          '[data-sidebar-resize]',
        );
        // Local grid/watchlist zooms must not counter a surrounding viewport
        // that is outside this animation and is not moving with them.
        const originElement =
          candidate && animatedElements.has(candidate) ? candidate : undefined;
        if (originElement && !origins.has(originElement)) {
          origins.set(originElement, {
            element: originElement,
            rect: visualRect(
              originElement,
              originElement.dataset.sidebarResize ?? 'xy',
            ),
            mode: originElement.dataset.sidebarResize ?? 'xy',
          });
        }
        const first =
          element.dataset.sidebarResize === 'media'
            ? (element.firstElementChild as HTMLElement | null)
            : null;
        const second = first?.nextElementSibling;
        const firstRect = first?.getBoundingClientRect();
        return {
          element,
          mode: element.dataset.sidebarResize ?? 'xy',
          rect: visualRect(element, element.dataset.sidebarResize ?? 'xy'),
          scrollTop: element.parentElement?.scrollTop ?? 0,
          origin: originElement ? origins.get(originElement) : undefined,
          mediaScale:
            first && firstRect
              ? firstRect.width / first.offsetWidth
              : undefined,
          mediaGap:
            second && firstRect
              ? second.getBoundingClientRect().left - firstRect.right
              : undefined,
          mediaHeight: firstRect?.height,
          exit: element.hasAttribute('data-sidebar-exit'),
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
  if (!positionX.has(mode)) transform.x = 0;
  if (!positionY.has(mode)) transform.y = 0;
  if (!['x', 'xy'].includes(mode)) transform.scaleX = 1;
  if (!['y', 'xy', 'y-scale', 'y-scale-scroll'].includes(mode))
    transform.scaleY = 1;
  if (mode === 'y-scale-scroll') {
    transform.y += finalScrollTop - transform.scaleY * previousScrollTop;
  }
  return transform;
}

export function sidebarResizePlans(
  snapshots: SidebarSnapshot[],
): AnimationPlan[] {
  if (prefersReducedMotion()) return [];
  const finals = new Map(
    snapshots.map((snapshot) => [
      snapshot.element,
      snapshot.element.getBoundingClientRect(),
    ]),
  );
  const mediaHeights = new Map(
    snapshots
      .filter((snapshot) => snapshot.mode === 'media')
      .map(({ element }) => [
        element,
        element.firstElementChild?.getBoundingClientRect().height,
      ]),
  );
  const zooms = new Map<Element, number>();
  const plans: AnimationPlan[] = [];
  const viewport = {
    top: 0,
    left: 0,
    right: innerWidth,
    bottom: innerHeight,
    width: innerWidth,
    height: innerHeight,
  };
  const worlds = new Map<HTMLElement, LayoutTransform[]>();
  const identity = { x: 0, y: 0, scaleX: 1, scaleY: 1 };
  const samples = 24;
  for (const snapshot of snapshots) {
    const {
      element,
      mode,
      scrollTop,
      origin,
      mediaScale,
      mediaGap,
      mediaHeight,
      enter,
      exit,
    } = snapshot;
    const final = finals.get(element)!;
    if (final.width <= 0 || final.height <= 0) continue;
    const rect = enter
      ? new DOMRect(
          final.left + final.width,
          final.top,
          final.width,
          final.height,
        )
      : snapshot.rect;
    let zoom = 1;
    if (mode === 'media') {
      zoom = Number.parseFloat(element.style.zoom) || 1;
    } else if (element.hasAttribute('data-sidebar-resize-zoom')) {
      const owner =
        element.closest('[data-youtube-watchlist-scale]') ?? element;
      if (!zooms.has(owner)) zooms.set(owner, mediaZoom(owner));
      zoom = zooms.get(owner)!;
    }
    const transform = sidebarResizeTransform(
      rect,
      final,
      mode,
      1,
      scrollTop,
      element.parentElement?.scrollTop ?? 0,
    );
    if (mode === 'media' && mediaScale) {
      transform.scaleX = mediaScale / zoom;
      transform.scaleY =
        mediaHeight && mediaHeights.get(element)
          ? mediaHeight / mediaHeights.get(element)!
          : rect.height / final.height;
    }
    if (mode === 'video') {
      const video = element as HTMLVideoElement;
      const aspect =
        video.videoWidth / video.videoHeight || final.width / final.height;
      const scale =
        Math.min(rect.width / aspect, rect.height) /
        Math.min(final.width / aspect, final.height);
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
    const parentFrames = origin ? worlds.get(origin.element) : undefined;
    const parentRect = origin ? finals.get(origin.element) : undefined;
    const finalMediaGap = (Number.parseFloat(element.style.gap) || 0) * zoom;
    const mediaColumns = Number(
      element.style.gridTemplateColumns.match(/^repeat\((\d+),/)?.[1] ?? 1,
    );
    const mediaWidth =
      Number.parseFloat(element.style.width) || final.width / zoom;
    const worldFrames: LayoutTransform[] = [];
    let changed = false;
    const keyframes = Array.from({ length: samples + 1 }, (_, index) => {
      const offset = index / samples;
      const remaining = 1 - offset;
      const parent = parentFrames?.[index] ?? identity;
      const inheritedX = parentRect
        ? parent.x + (final.left - parentRect.left) * (parent.scaleX - 1)
        : 0;
      const inheritedY = parentRect
        ? parent.y + (final.top - parentRect.top) * (parent.scaleY - 1)
        : 0;
      const controlsX = positionX.has(mode);
      const controlsY =
        positionY.has(mode) || ['y-scale', 'y-scale-scroll'].includes(mode);
      const world = {
        x: controlsX
          ? transform.x * remaining + (exit ? final.width * offset : 0)
          : inheritedX,
        y: controlsY ? transform.y * remaining : inheritedY,
        scaleX: controlsX
          ? 1 + (transform.scaleX - 1) * remaining
          : parent.scaleX,
        scaleY: controlsY
          ? 1 + (transform.scaleY - 1) * remaining
          : parent.scaleY,
      };
      worldFrames.push(world);
      const relative = {
        x: controlsX ? (world.x - inheritedX) / (parent.scaleX * zoom) : 0,
        y: controlsY ? (world.y - inheritedY) / (parent.scaleY * zoom) : 0,
        scaleX: controlsX ? world.scaleX / parent.scaleX : 1,
        scaleY: controlsY ? world.scaleY / parent.scaleY : 1,
      };
      changed ||=
        hasTransform(relative) ||
        Math.abs(relative.scaleX - 1) * final.width >= 0.5 ||
        Math.abs(relative.scaleY - 1) * final.height >= 0.5;
      const frame: Keyframe = { ...transformKeyframes(relative)[0], offset };
      if (mode === 'width' || mode === 'xy-width') {
        frame.width = `${(final.width + (rect.width - final.width) * remaining) / zoom}px`;
        changed ||= Math.abs(rect.width - final.width) >= 0.5;
      }
      if (mode === 'viewport') {
        frame.clipPath = `inset(0px ${final.width - (final.width + (rect.width - final.width) * remaining)}px 0px 0px)`;
        changed ||= Math.abs(rect.width - final.width) >= 0.5;
      }
      if (mode === 'media' && mediaGap !== undefined && mediaGap >= 0) {
        // Cards resize together; the screen-space gaps remain constant.
        const gap = finalMediaGap + (mediaGap - finalMediaGap) * remaining;
        const columnGap = gap / (zoom * world.scaleX);
        frame.columnGap = `${columnGap}px`;
        frame.width = `${mediaWidth + (mediaColumns - 1) * (columnGap - finalMediaGap / zoom)}px`;
      }
      return frame;
    });
    worlds.set(element, worldFrames);
    if (
      changed &&
      (isVisible(rect, viewport, 240) || isVisible(final, viewport, 240))
    )
      plans.push({ element, keyframes });
    if (
      element.hasAttribute('data-feed-content') &&
      element.hasAttribute('data-feed-scroll') &&
      Math.abs(transform.scaleX - 1) >= 0.001
    ) {
      for (const label of element.querySelectorAll<HTMLElement>(
        '.feed-group-label',
      )) {
        if (label.hasAttribute('data-sidebar-resize')) continue;
        const bounds = label.getBoundingClientRect();
        if (!isVisible(bounds, final, 240)) continue;
        plans.push({
          element: label,
          keyframes: [
            { '--feed-label-scale-x': transform.scaleX },
            { '--feed-label-scale-x': 1 },
          ],
        });
      }
    }
  }
  return plans;
}

// Register shared layout boundaries once, including content arriving after navigation.
// Media grids and horizontal rails own their children as a single animation group.
export function sidebarLayout(node: HTMLElement) {
  const owned = new Set<HTMLElement>();
  const selector =
    '.panel, .row, .statistics article, h1, h2, h3, h4, p, label, form, button, a, input, select, textarea, table, [data-sidebar-panel], span.ml-auto, span.flex, span.inline-flex, span.block, strong.block, small.block, .flex > span, .inline-flex > span, .grid > span, [role="dialog"]';
  function register() {
    for (const element of owned)
      if (!node.contains(element)) {
        element.removeAttribute('data-sidebar-resize');
        owned.delete(element);
      }
    const candidates = new Set(node.querySelectorAll<HTMLElement>(selector));
    for (const panel of node.querySelectorAll<HTMLElement>(
      '.panel, .row, .statistics article',
    )) {
      for (const child of panel.children)
        if (child instanceof HTMLElement && !child.matches('header, fieldset'))
          candidates.add(child);
    }
    for (const element of candidates) {
      if (
        element.hasAttribute('data-sidebar-resize') ||
        element.closest(
          '[data-card-grid], [data-sidebar-group], [data-youtube-watchlist-scale], .media-tile, .settings-navigation, [role="grid"]',
        )
      )
        continue;
      const mode = element.matches(
        '.panel, .row, .statistics article, [data-sidebar-panel], [role="dialog"]',
      )
        ? 'xy'
        : element.matches(
              'input:not([type=checkbox]):not([type=radio]), select, textarea, table',
            )
          ? 'xy-width'
          : 'xy-pos';
      element.dataset.sidebarResize = mode;
      owned.add(element);
    }
  }
  register();
  const observer = new MutationObserver(register);
  observer.observe(node, { childList: true, subtree: true });
  return {
    destroy() {
      observer.disconnect();
      for (const element of owned)
        element.removeAttribute('data-sidebar-resize');
    },
  };
}

export function createSidebarMotion() {
  let generation = 0;
  let animations: Animation[] = [];
  let clippedMain: HTMLElement | null = null;
  let viewports: { element: HTMLElement; overflow: string }[] = [];
  let connectedShell: HTMLElement | null = null;

  function restoreClip() {
    clippedMain?.style.removeProperty('overflow');
    clippedMain = null;
    for (const { element, overflow } of viewports) {
      if (overflow) element.style.overflow = overflow;
      else element.style.removeProperty('overflow');
    }
    viewports = [];
  }

  const motion = {
    connect(root: HTMLElement) {
      connectedShell = root;
      shellMotions.set(root, motion);
      return { destroy: () => motion.destroy() };
    },
    capture(
      root: HTMLElement | null,
      main: HTMLElement | null,
      watchlist?: WatchlistChange,
    ) {
      const current = ++generation;
      const snapshots = captureSidebarResize(root, !prefersReducedMotion());
      if (watchlist) {
        watchlist.element.toggleAttribute('data-sidebar-exit', watchlist.exit);
        const snapshot = snapshots.find(
          (snapshot) => snapshot.element === watchlist.element,
        );
        if (snapshot) {
          snapshot.enter = watchlist.enter;
          snapshot.exit = watchlist.exit;
        }
      }
      for (const animation of animations) animation.cancel();
      animations = [];
      restoreClip();
      clippedMain = main;
      if (main) main.style.overflow = 'visible';
      viewports = snapshots
        .filter((snapshot) => snapshot.mode === 'viewport')
        .map(({ element }) => ({ element, overflow: element.style.overflow }));
      for (const { element } of viewports) element.style.overflow = 'visible';
      return () => {
        if (current !== generation) return [];
        animations = playAnimations(sidebarResizePlans(snapshots));
        if (animations.length === 0) {
          restoreClip();
          root?.dispatchEvent(new Event('sidebar-motion-end'));
          return animations;
        }
        void Promise.allSettled(
          animations.map((animation) => animation.finished),
        ).then(() => {
          if (current !== generation) return;
          for (const { element, exit } of snapshots) {
            if (exit && element.hasAttribute('data-sidebar-exit')) {
              element.style.visibility = 'hidden';
              element.removeAttribute('data-sidebar-exit');
            }
          }
          animations = [];
          restoreClip();
          root?.dispatchEvent(new Event('sidebar-motion-end'));
        });
        return animations;
      };
    },
    destroy() {
      generation += 1;
      for (const animation of animations) animation.cancel();
      animations = [];
      restoreClip();
      if (connectedShell) shellMotions.delete(connectedShell);
      connectedShell = null;
    },
  };
  return motion;
}
