import { getContext, setContext } from 'svelte';
import {
  cancelLayoutAnimations,
  feedTransformPlans,
  hasTransform,
  playAnimations,
  animationsDisabled,
  rectTransform,
  transformKeyframes,
  type AnimationPlan,
  type LayoutRect,
  type PositionSnapshot,
} from './layout-animation';
import {
  captureSidebarMotion,
  captureSidebarResize,
  sidebarResizePlans,
} from './sidebar-motion';

export type CardKind = 'poster' | 'video';

const VIDEO_CARD_WIDTH_PX = 320;
// Poster pages usually fit three cards for every two on video pages. Laying
// posters out two-thirds as wide zooms both alike there, so their text matches.
export const CARD_KINDS: Record<
  CardKind,
  { logicalWidth: number; minColumns: number; maxColumns: number }
> = {
  poster: {
    logicalWidth: (VIDEO_CARD_WIDTH_PX * 2) / 3,
    minColumns: 5,
    maxColumns: 18,
  },
  video: {
    logicalWidth: VIDEO_CARD_WIDTH_PX,
    minColumns: 3,
    maxColumns: 12,
  },
};
export const CARD_GRID_GAP_PX = 12;
export const YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX = 430;
// Cards lay out at a fixed width and zoom to fill their column, so a 36 px
// channel avatar shows at a size that follows the column count.
export const cardAvatarSizes = (columns: number) =>
  `${((36 / CARD_KINDS.video.logicalWidth) * 100) / columns}vw`;
// `sizes="auto"` resolves to a card's width before its grid's zoom, which says
// nothing about the size it shows at. The column count does.
export const cardImageSizes = (columns: number) => `${100 / columns}vw`;

const gridColumnsKey = Symbol('card grid columns');

export function provideCardGridColumns(columns: () => number) {
  setContext(gridColumnsKey, columns);
}

export function cardGridColumns() {
  return getContext<(() => number) | undefined>(gridColumnsKey);
}

const mediaLayoutSynchronizers = new Set<() => void>();

export function syncMediaLayouts() {
  for (const synchronize of mediaLayoutSynchronizers) synchronize();
}

export interface CardGridMetrics {
  logicalWidth: number;
  logicalGap: number;
  logicalGridWidth: number;
  scale: number;
  inverseScale: number;
}

export interface YoutubeMediaMetrics extends CardGridMetrics {
  sidebarWidth: number;
}

export function nextCardColumns(
  current: number,
  deltaY: number,
  kind: CardKind,
): number {
  if (deltaY === 0) return current;
  const { minColumns, maxColumns } = CARD_KINDS[kind];
  // A count saved below the range can only step up into it.
  if (deltaY > 0)
    return Math.min(maxColumns, Math.max(minColumns, current + 1));
  return current > minColumns ? current - 1 : current;
}

export function cardGridTemplate(
  columns: number,
  logicalWidth: number,
): string {
  return `repeat(${columns},${logicalWidth}px)`;
}

export function cardGridMetrics(
  availableWidth: number,
  columns: number,
  kind: CardKind,
  gap = CARD_GRID_GAP_PX,
): CardGridMetrics | null {
  const { logicalWidth } = CARD_KINDS[kind];
  const usableWidth = availableWidth - gap * (columns - 1);
  if (usableWidth <= 0) return null;
  const renderedCardWidth = usableWidth / columns;
  const scale = renderedCardWidth / logicalWidth;
  if (scale <= 0) return null;
  const logicalGap = gap / scale;
  const logicalGridWidth = columns * logicalWidth + (columns - 1) * logicalGap;
  return {
    logicalWidth,
    logicalGap,
    logicalGridWidth,
    scale,
    inverseScale: 1 / scale,
  };
}

export function youtubeMediaMetrics(
  availableWidth: number,
  columns: number,
  sidebarOpen: boolean,
  kind: CardKind,
  scrollbarGutter = 0,
  gap = CARD_GRID_GAP_PX,
): YoutubeMediaMetrics | null {
  const { logicalWidth } = CARD_KINDS[kind];
  const fixedWidth = scrollbarGutter + gap * (columns - 1);
  const scopeWidth =
    columns * logicalWidth +
    (sidebarOpen ? YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX : 0);
  const scalableWidth = availableWidth - fixedWidth;
  if (scalableWidth <= 0 || scopeWidth <= 0) return null;
  const scale = scalableWidth / scopeWidth;
  if (scale <= 0) return null;
  const logicalGap = gap / scale;
  return {
    logicalWidth,
    logicalGap,
    logicalGridWidth: columns * logicalWidth + (columns - 1) * logicalGap,
    scale,
    inverseScale: 1 / scale,
    sidebarWidth: sidebarOpen ? YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX * scale : 0,
  };
}

function contentWidth(node: HTMLElement): number {
  const style = getComputedStyle(node);
  const left = Number.parseFloat(style.paddingLeft) || 0;
  const right = Number.parseFloat(style.paddingRight) || 0;
  return node.clientWidth - left - right;
}

function feedResizePlans(
  snapshot: PositionSnapshot | null,
  mode: 'scale' | 'width',
): AnimationPlan[] {
  if (!snapshot || animationsDisabled()) return [];
  const { element, rect } = snapshot;
  const final = element.getBoundingClientRect();
  if (final.width <= 0) return [];
  if (mode === 'width') {
    const widthDelta = final.width - rect.width;
    if (Math.abs(widthDelta) < 0.5) return [];
    // Individual cards already animate zoom. Keep the watchlist frame in place.
    return [
      {
        element,
        keyframes: [{ marginRight: `${widthDelta}px` }, { marginRight: '0px' }],
      },
    ];
  }
  const transform = rectTransform(rect, final);
  transform.y = 0;
  transform.scaleY = 1;
  return hasTransform(transform)
    ? feedTransformPlans(element, transform, final)
    : [];
}

type SidebarAnimationMode = 'scale' | 'open' | 'close';

function sidebarScalePlan(
  sidebar: HTMLElement,
  previousRect: LayoutRect | null,
  finalRect: LayoutRect,
  nextScale: number,
  mode: SidebarAnimationMode,
): AnimationPlan | null {
  if (
    !previousRect ||
    finalRect.width <= 0 ||
    finalRect.height <= 0 ||
    nextScale <= 0
  )
    return null;
  const transform = rectTransform(previousRect, finalRect, nextScale);
  const end =
    mode === 'close'
      ? `translateX(${YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX}px) scale(1, 1)`
      : undefined;
  return { element: sidebar, keyframes: transformKeyframes(transform, end) };
}

function applyGridMetrics(
  grids: HTMLElement[],
  columns: number,
  metrics: CardGridMetrics,
) {
  const template = cardGridTemplate(columns, metrics.logicalWidth);
  for (const grid of grids) {
    grid.style.gridTemplateColumns = template;
    // Rails place their cards past the visible columns.
    grid.style.gridAutoColumns = `${metrics.logicalWidth}px`;
    grid.style.width = `${metrics.logicalGridWidth}px`;
    grid.style.gap = `${metrics.logicalGap}px`;
    grid.style.zoom = String(metrics.scale);
    grid.style.setProperty(
      '--media-card-inverse-scale',
      String(metrics.inverseScale),
    );
  }
}

export interface CardScaleOptions {
  kind: CardKind;
  columns: number;
}

export function scaleCardScope(node: HTMLElement, initial: CardScaleOptions) {
  // The sidebar's scope sizes these grids together with its sidebar.
  if (node.closest('[data-sidebar-media-scope]')) return;
  let options = initial;
  let frame: number | null = null;
  let lastWidth: number | null = null;
  let grids: HTMLElement[] = [];

  function apply(width = contentWidth(node)) {
    const metrics = cardGridMetrics(width, options.columns, options.kind);
    if (!metrics) return;
    cancelLayoutAnimations(grids);
    lastWidth = width;
    applyGridMetrics(grids, options.columns, metrics);
  }

  function refreshGrids() {
    grids = [...node.querySelectorAll<HTMLElement>('[data-card-grid]')];
    apply();
  }

  function containsCardGrid(target: Node): boolean {
    return (
      target instanceof HTMLElement &&
      (target.hasAttribute('data-card-grid') ||
        target.querySelector('[data-card-grid]') !== null)
    );
  }

  function schedule() {
    if (frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      const width = contentWidth(node);
      if (lastWidth !== null && Math.abs(width - lastWidth) < 0.5) return;
      apply(width);
    });
  }

  const synchronize = () => apply(contentWidth(node));

  const resizeObserver = new ResizeObserver(schedule);
  const mutationObserver = new MutationObserver((records) => {
    if (
      records.some(
        (record) =>
          [...record.addedNodes].some(containsCardGrid) ||
          [...record.removedNodes].some(containsCardGrid),
      )
    ) {
      refreshGrids();
    }
  });
  resizeObserver.observe(node);
  mutationObserver.observe(node, { childList: true, subtree: true });
  mediaLayoutSynchronizers.add(synchronize);
  refreshGrids();

  return {
    update(next: CardScaleOptions) {
      if (next.columns === options.columns && next.kind === options.kind)
        return;
      options = next;
      apply();
    },
    destroy() {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      mediaLayoutSynchronizers.delete(synchronize);
      if (frame !== null) cancelAnimationFrame(frame);
      cancelLayoutAnimations(grids);
      for (const grid of grids) {
        grid.style.removeProperty('grid-template-columns');
        grid.style.removeProperty('grid-auto-columns');
        grid.style.removeProperty('width');
        grid.style.removeProperty('gap');
        grid.style.removeProperty('zoom');
        grid.style.removeProperty('--media-card-inverse-scale');
      }
    },
  };
}

export interface YoutubeScaleOptions extends CardScaleOptions {
  sidebarOpen: boolean;
}

export function scaleYoutubeMediaScope(
  node: HTMLElement,
  initial: YoutubeScaleOptions,
) {
  let options = initial;
  let frame: number | null = null;
  let lastWidth: number | null = null;
  let lastHeight: number | null = null;
  let grids: HTMLElement[] = [];
  let headers: HTMLElement[] = [];
  let feed: HTMLElement | null = null;
  let sidebarFrame: HTMLElement | null = null;
  let sidebarScale: HTMLElement | null = null;
  let sidebarAnimationGeneration = 0;

  function refreshTargets() {
    grids = [...node.querySelectorAll<HTMLElement>('[data-card-grid]')];
    headers = [
      ...node.querySelectorAll<HTMLElement>('[data-card-group-header]'),
    ];
    feed = node.querySelector<HTMLElement>('[data-feed-content]');
    sidebarFrame = node.querySelector<HTMLElement>(
      '[data-youtube-watchlist-frame]',
    );
    sidebarScale = node.querySelector<HTMLElement>(
      '[data-youtube-watchlist-scale]',
    );
  }

  // The feed's scrollbar and padding, which the cards don't scale into.
  function feedInset() {
    return feed ? Math.max(0, feed.offsetWidth - contentWidth(feed)) : 0;
  }

  function apply(
    width: number,
    height: number,
    gutter: number,
    animateGrid = false,
    sidebarAnimation: SidebarAnimationMode | null = null,
  ) {
    const metrics = youtubeMediaMetrics(
      width,
      options.columns,
      options.sidebarOpen,
      options.kind,
      gutter,
    );
    if (!metrics) return;
    const motionEnabled = !animationsDisabled();
    const feedSnapshot =
      motionEnabled && !animateGrid && sidebarAnimation === 'scale' && feed
        ? { element: feed, rect: feed.getBoundingClientRect() }
        : null;
    const sidebarWasHidden = sidebarScale
      ? getComputedStyle(sidebarScale).visibility === 'hidden'
      : false;
    const previousSidebarRect =
      sidebarAnimation && sidebarScale && !sidebarWasHidden && motionEnabled
        ? sidebarScale.getBoundingClientRect()
        : null;
    const animateSidebar =
      animateGrid && sidebarScale
        ? captureSidebarMotion(node, {
            element: sidebarScale,
            enter: sidebarAnimation === 'open' && sidebarWasHidden,
            exit: sidebarAnimation === 'close',
          })
        : undefined;
    const sidebarContent = animateSidebar
      ? []
      : captureSidebarResize(
          sidebarScale,
          Boolean(sidebarAnimation && !sidebarWasHidden && motionEnabled),
        );
    const feedContent =
      animateGrid && !animateSidebar
        ? captureSidebarResize(node, motionEnabled).filter(
            (snapshot) =>
              !snapshot.element.closest('[data-youtube-watchlist-frame]'),
          )
        : [];
    cancelLayoutAnimations([
      ...grids,
      ...headers,
      ...(feed ? [feed] : []),
      ...(sidebarScale ? [sidebarScale] : []),
    ]);
    lastWidth = width;
    lastHeight = height;
    applyGridMetrics(grids, options.columns, metrics);

    if (sidebarFrame) {
      sidebarFrame.style.width = `${metrics.sidebarWidth}px`;
      sidebarFrame.style.height = `${height}px`;
    }
    if (sidebarScale) {
      sidebarScale.style.visibility =
        options.sidebarOpen ||
        sidebarAnimation === 'close' ||
        sidebarScale.hasAttribute('data-sidebar-exit')
          ? 'visible'
          : 'hidden';
      sidebarScale.style.width = `${YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX}px`;
      sidebarScale.style.height = `${height / metrics.scale}px`;
      sidebarScale.style.zoom = String(metrics.scale);
      sidebarScale.style.transform = 'none';
      sidebarScale.style.transformOrigin = 'top left';
      sidebarScale.style.setProperty(
        '--youtube-media-scale',
        String(metrics.scale),
      );
      sidebarScale.style.setProperty(
        '--media-card-inverse-scale',
        String(metrics.inverseScale),
      );
    }

    const plans = [
      ...feedResizePlans(feedSnapshot, 'width'),
      ...sidebarResizePlans(feedContent),
      ...sidebarResizePlans(sidebarContent),
    ];
    let sidebarPlan: AnimationPlan | null = null;
    if (!animateSidebar && sidebarAnimation && sidebarScale && motionEnabled) {
      const finalRect = sidebarScale.getBoundingClientRect();
      const startRect =
        sidebarAnimation === 'open' && sidebarWasHidden
          ? new DOMRect(
              finalRect.left + finalRect.width,
              finalRect.top,
              finalRect.width,
              finalRect.height,
            )
          : previousSidebarRect;
      sidebarPlan = sidebarScalePlan(
        sidebarScale,
        startRect,
        finalRect,
        metrics.scale,
        sidebarAnimation,
      );
      if (sidebarPlan) plans.push(sidebarPlan);
    }
    const generation = ++sidebarAnimationGeneration;
    const animations = animateSidebar
      ? animateSidebar()
      : playAnimations(plans);
    if (sidebarAnimation === 'close' && sidebarScale) {
      const hide = () => {
        if (
          generation === sidebarAnimationGeneration &&
          !options.sidebarOpen &&
          sidebarScale
        ) {
          sidebarScale.style.visibility = 'hidden';
        }
      };
      const animation = animateSidebar
        ? animations.find(
            (animation) =>
              animation.effect instanceof KeyframeEffect &&
              animation.effect.target === sidebarScale,
          )
        : sidebarPlan
          ? animations[plans.indexOf(sidebarPlan)]
          : undefined;
      if (animation) void animation.finished.then(hide, () => undefined);
      else hide();
    }
  }

  function schedule() {
    if (frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      const width = node.clientWidth;
      const height = node.clientHeight;
      const gutter = feedInset();
      const widthChanged =
        lastWidth === null || Math.abs(width - lastWidth) >= 0.5;
      // Height only sizes the watchlist sidebar. Without it (mobile), the
      // pinned player's reveal must not re-apply the layout every frame.
      const heightChanged =
        sidebarFrame !== null &&
        (lastHeight === null || Math.abs(height - lastHeight) >= 0.5);
      if (!widthChanged && !heightChanged) return;
      apply(width, height, gutter);
    });
  }

  const synchronize = () =>
    apply(node.clientWidth, node.clientHeight, feedInset());

  function containsTarget(target: Node) {
    return (
      target instanceof HTMLElement &&
      (target.hasAttribute('data-card-grid') ||
        target.hasAttribute('data-card-group-header') ||
        target.hasAttribute('data-feed-content') ||
        target.hasAttribute('data-youtube-watchlist-frame') ||
        target.querySelector(
          '[data-card-grid], [data-card-group-header], [data-feed-content], [data-youtube-watchlist-frame]',
        ) !== null)
    );
  }

  const resizeObserver = new ResizeObserver(schedule);
  const mutationObserver = new MutationObserver((records) => {
    if (
      records.some(
        (record) =>
          [...record.addedNodes].some(containsTarget) ||
          [...record.removedNodes].some(containsTarget),
      )
    ) {
      refreshTargets();
      apply(node.clientWidth, node.clientHeight, feedInset());
    }
  });
  resizeObserver.observe(node);
  mutationObserver.observe(node, { childList: true, subtree: true });
  mediaLayoutSynchronizers.add(synchronize);
  refreshTargets();
  apply(node.clientWidth, node.clientHeight, feedInset());

  return {
    update(next: YoutubeScaleOptions) {
      const sidebarChanged = next.sidebarOpen !== options.sidebarOpen;
      const columnsChanged =
        next.columns !== options.columns || next.kind !== options.kind;
      if (!sidebarChanged && !columnsChanged) return;
      const sidebarAnimation: SidebarAnimationMode | null = sidebarChanged
        ? next.sidebarOpen
          ? 'open'
          : 'close'
        : columnsChanged && next.sidebarOpen
          ? 'scale'
          : null;
      options = next;
      const width = node.clientWidth;
      const height = node.clientHeight;
      const gutter = feedInset();
      apply(
        width,
        height,
        gutter,
        sidebarChanged && !columnsChanged,
        sidebarAnimation,
      );
    },
    destroy() {
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      mediaLayoutSynchronizers.delete(synchronize);
      if (frame !== null) cancelAnimationFrame(frame);
      sidebarAnimationGeneration += 1;
      captureSidebarResize(sidebarScale, false);
      cancelLayoutAnimations([
        ...grids,
        ...headers,
        ...(feed ? [feed] : []),
        ...(sidebarScale ? [sidebarScale] : []),
      ]);
      for (const grid of grids) {
        grid.style.removeProperty('grid-template-columns');
        grid.style.removeProperty('grid-auto-columns');
        grid.style.removeProperty('width');
        grid.style.removeProperty('gap');
        grid.style.removeProperty('zoom');
        grid.style.removeProperty('--media-card-inverse-scale');
      }
      sidebarFrame?.style.removeProperty('width');
      sidebarFrame?.style.removeProperty('height');
      if (sidebarScale) {
        sidebarScale.style.removeProperty('width');
        sidebarScale.style.removeProperty('height');
        sidebarScale.style.removeProperty('zoom');
        sidebarScale.style.removeProperty('transform');
        sidebarScale.style.removeProperty('transform-origin');
        sidebarScale.style.removeProperty('visibility');
        sidebarScale.style.removeProperty('--youtube-media-scale');
        sidebarScale.style.removeProperty('--media-card-inverse-scale');
      }
    },
  };
}
