import {
  cancelLayoutAnimations,
  layoutAnimationTiming,
  playAnimations,
  animationsDisabled,
  type AnimationPlan,
} from './layout-animation';
import { preparePage } from './page-readiness';
import type { AppRoute } from './navigation';

export function pageIdentity(route: AppRoute): string {
  return JSON.stringify([
    route.section,
    route.section === 'Settings' ? route.settings : null,
    route.service,
    route.workflow,
    route.detail,
    route.requests,
    route.item,
    route.ancestors,
    route.playlist,
    route.watchlist,
  ]);
}

// The leave animation usually covers loading. A page still loading after it
// shows a spinner, once the wait is long enough to notice.
const loadingIndicatorDelay = 150;

type Departure = {
  source: HTMLElement;
  viewport: HTMLElement;
  frame: HTMLElement;
  copy: HTMLElement;
};

// The copy renders in a closed shadow tree, so page code, layout motion, label
// and radio lookups, and tests never mistake it for the incoming page. It uses
// the document's styles, converted once per stylesheet.
const shadowSheets = new WeakMap<CSSStyleSheet, CSSStyleSheet>();
type RuleContainer = {
  cssRules: CSSRuleList;
  deleteRule(index: number): void;
};
// Tailwind styles `:host` like the root element. The copy must instead inherit
// the app's font and line height from where it sits in the page.
function withoutHost(container: RuleContainer) {
  for (let index = container.cssRules.length - 1; index >= 0; index--) {
    const rule = container.cssRules[index];
    if (rule instanceof CSSStyleRule && rule.selectorText.includes(':host')) {
      const selectors = rule.selectorText
        .split(',')
        .filter((selector) => !selector.includes(':host'));
      if (!selectors.length) {
        container.deleteRule(index);
        continue;
      }
      rule.selectorText = selectors.join(',');
    }
    if ('cssRules' in rule) withoutHost(rule as unknown as RuleContainer);
  }
}
function documentStyles() {
  return [...document.styleSheets].flatMap((sheet) => {
    let copy = shadowSheets.get(sheet);
    if (!copy)
      try {
        copy = new CSSStyleSheet({ media: sheet.media.mediaText });
        // `:root` never matches in a shadow tree; the copy's stand-in for the
        // document element answers to it instead, at the same specificity.
        copy.replaceSync(
          [...sheet.cssRules]
            .map((rule) => rule.cssText)
            .join('\n')
            .replaceAll(':root', ':is(:root, [data-leaving-root])'),
        );
        withoutHost(copy);
        shadowSheets.set(sheet, copy);
      } catch {
        return [];
      }
    return [copy];
  });
}

// A still copy of the outgoing content plays the leave animation, so the
// incoming page can mount and request its data underneath it straight away.
// It sits beside the original, fixed over the scroller, so neither scrolling
// nor the new content moves it.
function depart(source: HTMLElement): Departure {
  const viewport =
    source.closest<HTMLElement>('[data-feed-scroll]') ?? source.parentElement!;
  const bounds = viewport.getBoundingClientRect();
  const left = bounds.left + viewport.clientLeft;
  const top = bounds.top + viewport.clientTop;
  const rect = source.getBoundingClientRect();
  const copy = source.cloneNode(true) as HTMLElement;
  const originals = [source, ...source.querySelectorAll('*')];
  const copies = [copy, ...copy.querySelectorAll('*')];
  const scrolled: [Element, number, number][] = [];
  copies.forEach((element, index) => {
    const original = originals[index];
    if (original.scrollTop || original.scrollLeft)
      scrolled.push([element, original.scrollTop, original.scrollLeft]);
    // Cloning keeps typed input values but not choices made by script.
    if (
      original instanceof HTMLSelectElement &&
      element instanceof HTMLSelectElement
    )
      element.selectedIndex = original.selectedIndex;
  });
  for (const dialog of copy.querySelectorAll('dialog')) dialog.remove();
  Object.assign(copy.style, {
    position: 'absolute',
    left: `${rect.left - left}px`,
    top: `${rect.top - top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    margin: '0',
  });
  const frame = document.createElement('div');
  frame.dataset.pageLeaving = '';
  frame.inert = true;
  frame.setAttribute('aria-hidden', 'true');
  Object.assign(frame.style, {
    position: 'fixed',
    left: `${left}px`,
    top: `${top}px`,
    width: `${viewport.clientWidth}px`,
    height: `${viewport.clientHeight}px`,
    overflow: 'hidden',
    pointerEvents: 'none',
    zIndex: '1',
  });
  const shadow = frame.attachShadow({ mode: 'closed' });
  shadow.adoptedStyleSheets = documentStyles();
  // Selectors that look at ancestors, such as the desktop shell or settings
  // panels, need them inside the shadow tree too: boxless stand-ins carry
  // their classes and attributes.
  let parent: ParentNode = shadow;
  const ancestors: Element[] = [];
  for (
    let ancestor = source.parentElement;
    ancestor;
    ancestor = ancestor.parentElement
  )
    ancestors.unshift(ancestor);
  for (const ancestor of ancestors) {
    const stand = document.createElement('div');
    for (const { name, value } of ancestor.attributes)
      if (name !== 'id') stand.setAttribute(name, value);
    if (ancestor === document.documentElement) stand.dataset.leavingRoot = '';
    stand.style.display = 'contents';
    parent.append(stand);
    parent = stand;
  }
  parent.append(copy);
  source.after(frame);
  for (const [element, scrollTop, scrollLeft] of scrolled) {
    element.scrollTop = scrollTop;
    element.scrollLeft = scrollLeft;
  }
  return { source, viewport, frame, copy };
}

export class PageTransition {
  private root: HTMLElement | null = null;
  private surface: HTMLElement | null = null;
  private controller?: AbortController;
  private targets: HTMLElement[] = [];
  private loadingTimer?: ReturnType<typeof setTimeout>;
  private loadingShown = false;
  private departure?: Departure;

  // `loading` receives the area still loading, or null once it shows.
  constructor(
    private readonly loading: (surface: HTMLElement | null) => void = () => {},
  ) {}

  readonly connect = (root: HTMLElement) => {
    this.root = root;
    root.dataset.pagePhase = 'idle';
    return {
      destroy: () => {
        this.cancel();
        this.root = null;
      },
    };
  };

  private showLoading(immediately: boolean) {
    const show = () => {
      this.loadingShown = true;
      this.loading(this.surface);
    };
    if (immediately) show();
    else this.loadingTimer = setTimeout(show, loadingIndicatorDelay);
  }

  private hideLoading() {
    clearTimeout(this.loadingTimer);
    if (this.loadingShown) this.loading(null);
    this.loadingShown = false;
  }

  cancel() {
    this.controller?.abort();
    this.controller = undefined;
    this.hideLoading();
    cancelLayoutAnimations(this.targets);
    this.targets = [];
    this.departure?.frame.remove();
    this.departure = undefined;
    if (this.surface) {
      this.surface.style.removeProperty('opacity');
      this.surface.style.removeProperty('transform');
      this.surface.style.removeProperty('min-height');
      this.surface.inert = false;
      this.surface = null;
    }
    if (this.root) this.root.dataset.pagePhase = 'idle';
  }

  private plans(
    root: HTMLElement,
    entering: boolean,
    from?: { opacity: string; transform: string },
    viewport = root.closest<HTMLElement>('[data-feed-scroll]') ?? root,
  ): AnimationPlan[] {
    const bounds = viewport.getBoundingClientRect();
    const rect = root.getBoundingClientRect();
    const center = {
      x: bounds.left + bounds.width / 2,
      y: bounds.top + bounds.height / 2,
    };
    root.style.transformOrigin = `${center.x - rect.left}px ${center.y - rect.top}px`;
    const sidebar = entering
      ? root.querySelector<HTMLElement>(
          '[data-page-slide][aria-hidden="false"]',
        )
      : null;
    const plans: AnimationPlan[] = [
      {
        element: root,
        keyframes: entering
          ? [
              { opacity: 0, transform: sidebar ? 'none' : 'scale(0.9)' },
              { opacity: 1, transform: 'none' },
            ]
          : [
              {
                opacity: from?.opacity ?? '1',
                transform: from?.transform ?? 'none',
              },
              { opacity: 0, transform: 'scale(0.9)' },
            ],
        options: { fill: entering ? 'backwards' : 'forwards' },
      },
    ];
    if (sidebar) {
      for (const element of root.querySelectorAll<HTMLElement>(
        '[data-page-scale]',
      )) {
        const rect = element.getBoundingClientRect();
        plans.push({
          element,
          keyframes: [
            {
              transform: 'scale(0.9)',
              transformOrigin: `${center.x - rect.left}px ${center.y - rect.top}px`,
            },
            {
              transform: 'none',
              transformOrigin: `${center.x - rect.left}px ${center.y - rect.top}px`,
            },
          ],
        });
      }
      plans.push({
        element: sidebar,
        keyframes: [
          { transform: 'translateX(100%)' },
          { transform: 'translateX(0)' },
        ],
      });
    }
    this.targets = plans.map((plan) => plan.element);
    return plans;
  }

  async change(
    update: () => Promise<void>,
    restore: () => Promise<void>,
    region?: 'settings' | 'service',
  ) {
    const root = this.root;
    const content = () =>
      (region &&
        root?.querySelector<HTMLElement>(`[data-page-region="${region}"]`)) ||
      (region &&
        root?.querySelector<HTMLElement>('[data-page-region="settings"]')) ||
      root;
    let surface = content();
    // Navigating again during the leave animation keeps fading the same copy.
    const previous = this.departure;
    this.departure = undefined;
    const outgoing = previous?.source === surface ? previous.copy : surface;
    const from = outgoing
      ? {
          opacity: getComputedStyle(outgoing).opacity,
          transform: getComputedStyle(outgoing).transform,
        }
      : undefined;
    // Navigating again while a spinner shows keeps it in place.
    const loadingShown = this.loadingShown;
    this.cancel();
    this.surface = surface;
    const controller = new AbortController();
    this.controller = controller;
    const { signal } = controller;
    let preparation: ReturnType<typeof preparePage> | undefined;
    let focused: HTMLElement | undefined;
    try {
      let leaving: Promise<unknown> = Promise.resolve();
      if (root && surface) {
        root.dataset.pagePhase = 'leaving';
        surface.dispatchEvent(
          new Event('thelxinoe-page-leave', { bubbles: true }),
        );
        if (Number(from?.opacity ?? 1) > 0 && !animationsDisabled()) {
          this.departure =
            previous?.source === surface ? previous : depart(surface);
          // An interrupted leave animation only ends the leave early.
          leaving = Promise.all(
            playAnimations(
              this.plans(
                this.departure.copy,
                false,
                from,
                this.departure.viewport,
              ),
              layoutAnimationTiming,
            ).map((animation) => animation.finished),
          ).catch(() => {});
        }
        surface.style.opacity = '0';
        surface.style.transform = 'none';
        // The incoming page starts out shorter; keeping the height until the
        // leave ends stops the scroller clamping what stays on screen.
        if (this.departure)
          surface.style.minHeight = `${surface.getBoundingClientRect().height}px`;
        if (
          document.activeElement instanceof HTMLElement &&
          surface.contains(document.activeElement)
        )
          focused = document.activeElement;
        surface.inert = true;
      }
      if (previous && previous !== this.departure) previous.frame.remove();
      // The incoming page mounts now, hidden, so its requests run during the
      // leave animation instead of after it.
      preparation = preparePage();
      const mounted = update().then(() => {
        signal.throwIfAborted();
        const incoming = content();
        if (incoming && incoming !== surface) {
          surface?.style.removeProperty('opacity');
          surface?.style.removeProperty('transform');
          surface?.style.removeProperty('min-height');
          if (surface) surface.inert = false;
          surface = incoming;
          this.surface = surface;
          surface.style.opacity = '0';
          surface.inert = true;
          if (this.loadingShown) this.loading(surface);
        }
      });
      void mounted.catch(() => {});
      await leaving;
      signal.throwIfAborted();
      cancelLayoutAnimations(this.targets);
      this.targets = [];
      this.departure?.frame.remove();
      this.departure = undefined;
      surface?.style.removeProperty('min-height');
      if (root && surface) {
        root.dataset.pagePhase = 'loading';
        this.showLoading(loadingShown);
      }
      await mounted;
      signal.throwIfAborted();
      // Restoring now, without the held height, keeps the saved position until
      // the incoming content is tall enough for it.
      await restore();
      const viewport = root?.closest<HTMLElement>('[data-feed-scroll]') ?? root;
      if (surface && viewport)
        await preparation.ready(surface, viewport, signal);
      signal.throwIfAborted();
      preparation.close();
      this.hideLoading();
      if (root && surface) {
        root.dataset.pagePhase = 'entering';
        surface.style.opacity = '1';
        await Promise.all(
          playAnimations(this.plans(surface, true), layoutAnimationTiming).map(
            (animation) => animation.finished,
          ),
        );
        signal.throwIfAborted();
        cancelLayoutAnimations(this.targets);
        this.targets = [];
        surface.style.removeProperty('opacity');
        surface.style.removeProperty('transform');
        surface.inert = false;
        this.surface = null;
        if (focused?.isConnected && document.activeElement === document.body)
          focused.focus({ preventScroll: true });
        root.dataset.pagePhase = 'idle';
      }
      return true;
    } catch (error) {
      if (!signal.aborted) {
        this.cancel();
        throw error;
      }
      return false;
    } finally {
      preparation?.close();
      if (this.controller === controller) this.controller = undefined;
    }
  }
}
