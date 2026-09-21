import {
  cancelLayoutAnimations,
  capturePositions,
  feedTransformPlans,
  hasTransform,
  playAnimations,
  prefersReducedMotion,
  rectTransform,
  transformKeyframes,
  type AnimationPlan,
  type LayoutRect,
  type PositionSnapshot,
} from './layout-animation';
import { captureSidebarResize, sidebarResizePlans } from './sidebar-motion';

const MIN_CARD_COLUMNS = 3;
const MAX_CARD_COLUMNS = 12;

export const DEFAULT_CARD_COLUMNS = 6;
export const CARD_LOGICAL_WIDTH_PX = 320;
export const CARD_GRID_GAP_PX = 12;
export const YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX = 430;
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

export function parseCardColumns(value: string | null): number | null {
  if (value === null || value.trim() === '') return null;
  const columns = Number(value);
  return Number.isInteger(columns) &&
    columns >= MIN_CARD_COLUMNS &&
    columns <= MAX_CARD_COLUMNS
    ? columns
    : null;
}

export function nextCardColumns(current: number, deltaY: number): number {
  if (deltaY === 0) return current;
  const delta = deltaY > 0 ? 1 : -1;
  return Math.min(
    MAX_CARD_COLUMNS,
    Math.max(MIN_CARD_COLUMNS, current + delta),
  );
}

export function cardGridTemplate(columns: number): string {
  return `repeat(${columns},${CARD_LOGICAL_WIDTH_PX}px)`;
}

export function cardGridMetrics(
  availableWidth: number,
  columns: number,
  gap = CARD_GRID_GAP_PX,
): CardGridMetrics | null {
  const usableWidth = availableWidth - gap * (columns - 1);
  if (usableWidth <= 0) return null;
  const renderedCardWidth = usableWidth / columns;
  const scale = renderedCardWidth / CARD_LOGICAL_WIDTH_PX;
  if (scale <= 0) return null;
  const logicalGap = gap / scale;
  const logicalGridWidth =
    columns * CARD_LOGICAL_WIDTH_PX + (columns - 1) * logicalGap;
  return {
    logicalWidth: CARD_LOGICAL_WIDTH_PX,
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
  scrollbarGutter = 0,
  gap = CARD_GRID_GAP_PX,
): YoutubeMediaMetrics | null {
  const fixedWidth = scrollbarGutter + gap * (columns - 1);
  const logicalWidth =
    columns * CARD_LOGICAL_WIDTH_PX +
    (sidebarOpen ? YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX : 0);
  const scalableWidth = availableWidth - fixedWidth;
  if (scalableWidth <= 0 || logicalWidth <= 0) return null;
  const scale = scalableWidth / logicalWidth;
  if (scale <= 0) return null;
  const logicalGap = gap / scale;
  return {
    logicalWidth: CARD_LOGICAL_WIDTH_PX,
    logicalGap,
    logicalGridWidth:
      columns * CARD_LOGICAL_WIDTH_PX + (columns - 1) * logicalGap,
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
  if (!snapshot || prefersReducedMotion()) return [];
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

function gridFlipPlans(
  snapshots: PositionSnapshot[],
  zoom: number,
  horizontal: boolean,
): AnimationPlan[] {
  return snapshots.flatMap(({ element, rect }) => {
    const final = element.getBoundingClientRect();
    if (final.width <= 0 || final.height <= 0) return [];
    const transform = rectTransform(rect, final, zoom);
    if (!horizontal) {
      transform.x = 0;
      transform.scaleX = 1;
    }
    return hasTransform(transform)
      ? [{ element, keyframes: transformKeyframes(transform) }]
      : [];
  });
}

function headerFlipPlans(
  snapshots: PositionSnapshot[],
  horizontal: boolean,
): AnimationPlan[] {
  return snapshots.flatMap(({ element, rect }) => {
    const transform = rectTransform(rect, element.getBoundingClientRect());
    if (!horizontal) transform.x = 0;
    transform.scaleX = 1;
    transform.scaleY = 1;
    return hasTransform(transform)
      ? [{ element, keyframes: transformKeyframes(transform) }]
      : [];
  });
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
  const template = cardGridTemplate(columns);
  for (const grid of grids) {
    grid.style.gridTemplateColumns = template;
    grid.style.width = `${metrics.logicalGridWidth}px`;
    grid.style.gap = `${metrics.logicalGap}px`;
    grid.style.zoom = String(metrics.scale);
    grid.style.setProperty(
      '--media-card-inverse-scale',
      String(metrics.inverseScale),
    );
  }
}

export function scaleCardScope(node: HTMLElement, initialColumns: number) {
  let columns = initialColumns;
  let frame: number | null = null;
  let lastWidth: number | null = null;
  let grids: HTMLElement[] = [];

  function apply(width = contentWidth(node)) {
    const metrics = cardGridMetrics(width, columns);
    if (!metrics) return;
    cancelLayoutAnimations(grids);
    lastWidth = width;
    applyGridMetrics(grids, columns, metrics);
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
    update(nextColumns: number) {
      if (nextColumns === columns) return;
      columns = nextColumns;
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
        grid.style.removeProperty('width');
        grid.style.removeProperty('gap');
        grid.style.removeProperty('zoom');
        grid.style.removeProperty('--media-card-inverse-scale');
      }
    },
  };
}

export interface YoutubeScaleOptions {
  columns: number;
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

  function scrollbarGutter() {
    return feed ? Math.max(0, feed.offsetWidth - feed.clientWidth) : 0;
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
      gutter,
    );
    if (!metrics) return;
    const motionEnabled = !prefersReducedMotion();
    const feedSnapshot =
      motionEnabled && (animateGrid || sidebarAnimation === 'scale') && feed
        ? { element: feed, rect: feed.getBoundingClientRect() }
        : null;
    const gridSnapshots =
      animateGrid && motionEnabled ? capturePositions(grids) : [];
    const headerSnapshots =
      animateGrid && motionEnabled ? capturePositions(headers) : [];
    const sidebarWasHidden = sidebarScale
      ? getComputedStyle(sidebarScale).visibility === 'hidden'
      : false;
    const previousSidebarRect =
      sidebarAnimation && sidebarScale && !sidebarWasHidden && motionEnabled
        ? sidebarScale.getBoundingClientRect()
        : null;
    const sidebarContent = captureSidebarResize(
      sidebarScale,
      Boolean(sidebarAnimation && !sidebarWasHidden && motionEnabled),
    );
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
      sidebarFrame.style.height = `max(0px, calc(var(--workspace-height, ${height}px) - 12px))`;
    }
    if (sidebarScale) {
      sidebarScale.style.visibility =
        options.sidebarOpen || sidebarAnimation === 'close'
          ? 'visible'
          : 'hidden';
      sidebarScale.style.width = `${YOUTUBE_SIDEBAR_LOGICAL_WIDTH_PX}px`;
      sidebarScale.style.height = `max(0px, calc((var(--workspace-height, ${height}px) - 12px) / ${metrics.scale}))`;
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
      ...feedResizePlans(feedSnapshot, animateGrid ? 'scale' : 'width'),
      ...gridFlipPlans(gridSnapshots, metrics.scale, !feedSnapshot),
      ...headerFlipPlans(headerSnapshots, !feedSnapshot),
      ...sidebarResizePlans(sidebarContent),
    ];
    let sidebarPlan: AnimationPlan | null = null;
    if (sidebarAnimation && sidebarScale && motionEnabled) {
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
    const animations = playAnimations(plans);
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
      const animation = sidebarPlan
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
      const gutter = scrollbarGutter();
      const widthChanged =
        lastWidth === null || Math.abs(width - lastWidth) >= 0.5;
      const heightChanged =
        lastHeight === null || Math.abs(height - lastHeight) >= 0.5;
      if (!widthChanged && !heightChanged) return;
      apply(width, height, gutter);
    });
  }

  const synchronize = () =>
    apply(node.clientWidth, node.clientHeight, scrollbarGutter());

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
      apply(node.clientWidth, node.clientHeight, scrollbarGutter());
    }
  });
  resizeObserver.observe(node);
  mutationObserver.observe(node, { childList: true, subtree: true });
  mediaLayoutSynchronizers.add(synchronize);
  refreshTargets();
  apply(node.clientWidth, node.clientHeight, scrollbarGutter());

  return {
    update(next: YoutubeScaleOptions) {
      const sidebarChanged = next.sidebarOpen !== options.sidebarOpen;
      const columnsChanged = next.columns !== options.columns;
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
      const gutter = scrollbarGutter();
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
