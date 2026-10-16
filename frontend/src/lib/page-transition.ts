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

export class PageTransition {
  private root: HTMLElement | null = null;
  private surface: HTMLElement | null = null;
  private controller?: AbortController;
  private targets: HTMLElement[] = [];

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

  cancel() {
    this.controller?.abort();
    this.controller = undefined;
    cancelLayoutAnimations(this.targets);
    this.targets = [];
    if (this.surface) {
      this.surface.style.removeProperty('opacity');
      this.surface.style.removeProperty('transform');
      this.surface.inert = false;
      this.surface = null;
    }
    if (this.root) this.root.dataset.pagePhase = 'idle';
  }

  private plans(
    root: HTMLElement,
    entering: boolean,
    from?: { opacity: string; transform: string },
  ): AnimationPlan[] {
    const viewport = root.closest<HTMLElement>('[data-feed-scroll]') ?? root;
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
    const from = surface
      ? {
          opacity: getComputedStyle(surface).opacity,
          transform: getComputedStyle(surface).transform,
        }
      : undefined;
    this.cancel();
    this.surface = surface;
    const controller = new AbortController();
    this.controller = controller;
    const { signal } = controller;
    let preparation: ReturnType<typeof preparePage> | undefined;
    let focused: HTMLElement | undefined;
    try {
      if (root && surface) {
        root.dataset.pagePhase = 'leaving';
        surface.dispatchEvent(
          new Event('thelxinoe-page-leave', { bubbles: true }),
        );
        if (Number(from?.opacity ?? 1) > 0 && !animationsDisabled())
          await Promise.all(
            playAnimations(
              this.plans(surface, false, from),
              layoutAnimationTiming,
            ).map((animation) => animation.finished),
          );
        signal.throwIfAborted();
        surface.style.opacity = '0';
        surface.style.transform = 'none';
        cancelLayoutAnimations(this.targets);
        if (
          document.activeElement instanceof HTMLElement &&
          surface.contains(document.activeElement)
        )
          focused = document.activeElement;
        surface.inert = true;
        root.dataset.pagePhase = 'loading';
      }
      preparation = preparePage();
      await update();
      signal.throwIfAborted();
      const incoming = content();
      if (incoming && incoming !== surface) {
        surface?.style.removeProperty('opacity');
        surface?.style.removeProperty('transform');
        if (surface) surface.inert = false;
        surface = incoming;
        this.surface = surface;
        surface.style.opacity = '0';
        surface.inert = true;
      }
      const viewport = root?.closest<HTMLElement>('[data-feed-scroll]') ?? root;
      await restore();
      if (surface && viewport)
        await preparation.ready(surface, viewport, signal);
      signal.throwIfAborted();
      preparation.close();
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
