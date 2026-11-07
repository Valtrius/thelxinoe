import { tick } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import {
  isVisible,
  playAnimations,
  type AnimationPlan,
  type LayoutRect,
} from '../layout-animation';
import { getAnimationDuration } from '../motion';
import { pagePreparing } from '../page-readiness';

// Rows show they are stale only once a read outlasts this, so that a fast
// response never flashes a loading state. quietDisabledClass matches it.
const staleDelay = 200;

// Linear, so that these are real time: arrivals start once moving rows have
// nearly landed and leaving ones have gone.
const fadeOutEnds = 0.4;
const fadeInStarts = 0.35;
const scrollingBackEasing = 'cubic-bezier(0.65, 0, 0.35, 1)';

export type RowChange = 'none' | 'move' | 'arrive' | 'jump' | 'leave';

/** How a row that is still listed gets from where it was to where it is. */
export function rowChange(
  was: LayoutRect | undefined,
  now: LayoutRect,
  viewport: LayoutRect,
): RowChange {
  const onScreen = isVisible(now, viewport);
  if (!was || !isVisible(was, viewport)) return onScreen ? 'arrive' : 'none';
  const y = Math.abs(was.top - now.top);
  // Crossing more than a screen would streak, so it fades out at its old place
  // and in at its new one instead.
  if (y > viewport.height) return onScreen ? 'jump' : 'leave';
  return y >= 0.5 || Math.abs(was.left - now.left) >= 0.5 ? 'move' : 'none';
}

/** Flattens computed rgb() and rgba() backgrounds, nearest first, into one. */
export function flattenColors(layers: string[]): string {
  const parsed = layers
    .filter((layer) => layer.startsWith('rgb'))
    .map((layer) => (layer.match(/[\d.]+/g) ?? []).map(Number))
    .map(([red, green, blue, alpha = 1]) => ({
      channels: [red, green, blue],
      alpha,
    }));
  const opaque = parsed.findIndex((layer) => layer.alpha === 1);
  const visible = opaque < 0 ? parsed : parsed.slice(0, opaque + 1);
  const flat = visible.reduceRight(
    (under, { channels, alpha }) =>
      channels.map((channel, index) =>
        Math.round(channel * alpha + under[index] * (1 - alpha)),
      ),
    [255, 255, 255],
  );
  return `rgb(${flat.join(' ')})`;
}

type Capture = {
  key: string;
  element: HTMLElement;
  rect: DOMRect;
  table: HTMLTableElement | null;
  /** Its own opacity, below 1 when a fade-in was interrupted. */
  opacity: number;
  /** The opacity on screen, which also has the list dimming it. */
  seen: number;
};

function capture(content: HTMLElement) {
  const rows = new SvelteMap<string, Capture>();
  const list = Number(getComputedStyle(content).opacity);
  for (const element of content.querySelectorAll<HTMLElement>(
    '[data-motion-row]',
  )) {
    const key = element.dataset.motionRow!;
    const opacity = Number(getComputedStyle(element).opacity);
    rows.set(key, {
      key,
      element,
      rect: element.getBoundingClientRect(),
      table: element.closest('table'),
      opacity,
      seen: opacity * list,
    });
  }
  return rows;
}

function scrollerOf(root: HTMLElement) {
  for (let parent = root.parentElement; parent; parent = parent.parentElement) {
    const { overflowY } = getComputedStyle(parent);
    if (overflowY === 'auto' || overflowY === 'scroll') return parent;
  }
  return document.documentElement;
}

/**
 * Takes the list from its old height to its new one. Getting shorter starts from
 * the bottom of the screen, since what is past it can go unseen. Going through
 * it would use up the animation, and leave the page to scroll back in one frame.
 * That scrolling happens right at the start, so it starts slowly when it is due.
 */
function resize(
  root: HTMLElement,
  from: number,
  to: number,
): AnimationPlan | undefined {
  const scroller = scrollerOf(root);
  const onScreen =
    scroller.getBoundingClientRect().bottom - root.getBoundingClientRect().top;
  const start = to < from ? Math.max(to, Math.min(from, onScreen)) : from;
  if (Math.abs(to - start) < 1) return undefined;
  return {
    element: root,
    keyframes: [{ height: `${start}px` }, { height: `${to}px` }],
    options:
      to < onScreen && scroller.scrollTop > 0
        ? { easing: scrollingBackEasing }
        : undefined,
  };
}

/** The colour rows have over the page, so that moving ones can cover what they
 *  pass over without changing colour. */
function surfaceOf(root: HTMLElement) {
  const layers: string[] = [];
  for (let node: HTMLElement | null = root; node; node = node.parentElement)
    layers.push(getComputedStyle(node).backgroundColor);
  return flattenColors(layers);
}

function inertCopy<T extends HTMLElement>(element: T): T {
  const copy = element.cloneNode(true) as T;
  // A clone starts at the end of its animations, which is not where they are.
  copy.querySelectorAll('[data-motion-layer]').forEach((layer) => {
    layer.remove();
  });
  const selects = element.querySelectorAll('select');
  copy.querySelectorAll('select').forEach((select, index) => {
    select.value = selects[index].value;
  });
  for (const node of [
    copy,
    ...copy.querySelectorAll('[id], [data-motion-row]'),
  ]) {
    node.removeAttribute('id');
    node.removeAttribute('data-motion-row');
  }
  return copy;
}

/** A removed row where it was, inside a table with the same columns. */
function ghostOf({ element, rect, table }: Capture, origin: DOMRect) {
  const copy = inertCopy(element);
  let ghost = copy;
  if (table && ['TR', 'TBODY', 'THEAD'].includes(element.tagName)) {
    const wrapper = document.createElement('table');
    wrapper.className = table.className;
    const columns = table.querySelector('colgroup');
    if (columns) wrapper.append(columns.cloneNode(true));
    if (element.tagName === 'TR') {
      const body = document.createElement('tbody');
      body.className = element.parentElement?.className ?? '';
      body.append(copy);
      wrapper.append(body);
    } else wrapper.append(copy);
    ghost = wrapper;
  }
  ghost.style.position = 'absolute';
  ghost.style.top = `${rect.top - origin.top}px`;
  ghost.style.left = `${rect.left - origin.left}px`;
  ghost.style.width = `${rect.width}px`;
  return ghost;
}

/**
 * Moves rows from what is on screen to what a read returned. Rows carry
 * `data-motion-row` with a key that stays the same wherever they are listed.
 * Render them inside a RowMotionBody, and apply each result through `commit`.
 */
export class RowMotion {
  root = $state<HTMLElement>();
  content = $state<HTMLElement>();
  /** A read that will replace the rows is under way. */
  holding = $state(false);
  /** It has taken long enough to show. */
  stale = $state(false);
  private timer?: ReturnType<typeof setTimeout>;
  private running: Animation[] = [];
  private layer?: HTMLElement;
  private leaving: { item: Capture; copy: HTMLElement }[] = [];
  private turn = 0;

  hold() {
    this.holding = true;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => (this.stale = true), staleDelay);
  }

  release() {
    this.holding = false;
    clearTimeout(this.timer);
    this.stale = false;
  }

  /**
   * Applies `update`, animating from what is on screen right now. `live` marks
   * new rows as having just appeared. `dissolve` fades between the two states
   * instead of moving rows, for content they have nothing in common with.
   */
  async commit(
    update: () => void,
    { live = false, dissolve = false } = {},
  ): Promise<void> {
    const skeletonShown = this.stale;
    this.release();
    const { root, content } = this;
    const turn = ++this.turn;
    if (!root || !content || !getAnimationDuration() || pagePreparing()) {
      update();
      return;
    }
    // Reading first keeps an interrupted animation going from where it is.
    const before = capture(content);
    for (const { item, copy } of this.leaving) {
      if (before.has(item.key)) continue;
      const seen = Number(getComputedStyle(copy).opacity);
      before.set(item.key, {
        ...item,
        rect: copy.getBoundingClientRect(),
        opacity: seen,
        seen,
      });
    }
    // A first read has nothing on screen to animate from, unless a skeleton is.
    if (!before.size && !skeletonShown && !dissolve) {
      update();
      return;
    }
    const from = root.getBoundingClientRect().height;
    const previous = dissolve || !before.size ? inertCopy(content) : null;
    this.settle();
    // Holding the height stops the scroller clamping before the animation runs.
    root.style.height = `${from}px`;
    root.style.overflow = 'clip';
    update();
    await tick();
    if (turn !== this.turn || !root.isConnected) return;
    const to = content.getBoundingClientRect().height;
    const plans: AnimationPlan[] = [];
    const resizing = resize(root, from, to);
    if (resizing) plans.push(resizing);
    if (previous) this.dissolve(root, content, previous, plans);
    else this.move(root, content, before, plans, live);
    this.play(root, plans, to);
  }

  private dissolve(
    root: HTMLElement,
    content: HTMLElement,
    previous: HTMLElement,
    plans: AnimationPlan[],
  ) {
    previous.style.position = 'absolute';
    previous.style.inset = '0 0 auto 0';
    this.addLayer(root).append(previous);
    plans.push(
      { element: previous, keyframes: [{ opacity: 1 }, { opacity: 0 }] },
      { element: content, keyframes: [{ opacity: 0 }, { opacity: 1 }] },
    );
  }

  private move(
    root: HTMLElement,
    content: HTMLElement,
    before: Map<string, Capture>,
    plans: AnimationPlan[],
    live: boolean,
  ) {
    const viewport = scrollerOf(root).getBoundingClientRect();
    const origin = root.getBoundingClientRect();
    const surface = surfaceOf(root);
    const leaving: Capture[] = [];
    for (const element of content.querySelectorAll<HTMLElement>(
      '[data-motion-row]',
    )) {
      const key = element.dataset.motionRow!;
      const was = before.get(key);
      before.delete(key);
      const rect = element.getBoundingClientRect();
      const change = rowChange(was?.rect, rect, viewport);
      if (change === 'move') {
        const x = was!.rect.left - rect.left;
        const y = was!.rect.top - rect.top;
        // Over the page's colour, so that a highlighted row stays highlighted.
        const backgroundColor = flattenColors([
          getComputedStyle(element).backgroundColor,
          surface,
        ]);
        plans.push({
          element,
          keyframes: [
            { transform: `translate(${x}px, ${y}px)`, backgroundColor },
            { transform: 'none', backgroundColor },
          ],
        });
      }
      if (was && was.opacity < 1 && (change === 'move' || change === 'none'))
        plans.push({
          element,
          keyframes: [{ opacity: was.opacity }, { opacity: 1 }],
          options: { easing: 'linear' },
        });
      if (change === 'jump' || change === 'leave') leaving.push(was!);
      if (change === 'jump' || change === 'arrive') {
        plans.push({
          element,
          keyframes: [
            { opacity: 0 },
            { opacity: 0, offset: fadeInStarts },
            { opacity: 1 },
          ],
          options: { easing: 'linear' },
        });
        if (live && !was)
          element.animate(
            [
              {
                backgroundColor:
                  'color-mix(in srgb, var(--accent) 16%, transparent)',
              },
              { backgroundColor: 'transparent' },
            ],
            { duration: 1400, easing: 'ease-out' },
          );
      }
    }
    // What is left is no longer listed.
    for (const item of before.values())
      if (isVisible(item.rect, viewport)) leaving.push(item);
    const fading = leaving.filter((item) => item.seen > 0);
    if (!fading.length) return;
    const layer = this.addLayer(root);
    for (const item of fading) {
      const ghost = ghostOf(item, origin);
      layer.append(ghost);
      this.leaving.push({ item, copy: ghost });
      plans.push({
        element: ghost,
        keyframes: [
          { opacity: item.seen },
          { opacity: 0, offset: fadeOutEnds },
          { opacity: 0 },
        ],
        options: { easing: 'linear' },
      });
    }
  }

  // Comes before the rows, so that moving ones pass over what is leaving.
  private addLayer(root: HTMLElement) {
    const layer = document.createElement('div');
    layer.setAttribute('aria-hidden', 'true');
    layer.dataset.motionLayer = '';
    layer.inert = true;
    layer.className = 'pointer-events-none absolute inset-0';
    root.prepend(layer);
    this.layer = layer;
    return layer;
  }

  private play(root: HTMLElement, plans: AnimationPlan[], height: number) {
    const animations = playAnimations(plans);
    this.running = animations;
    if (!animations.length) {
      this.clear(root);
      return;
    }
    // Only now, with the height animating: any sooner and the next layout would
    // already be at the new height, and scroll the page there.
    root.style.height = `${height}px`;
    void Promise.allSettled(animations.map((item) => item.finished)).then(
      () => {
        if (this.running === animations) this.clear(root);
      },
    );
  }

  private clear(root: HTMLElement) {
    this.layer?.remove();
    this.layer = undefined;
    this.leaving = [];
    root.style.height = '';
    root.style.overflow = '';
  }

  /** Ends the current animation at once, for when its rows are being replaced. */
  settle() {
    const running = this.running;
    this.running = [];
    for (const animation of running) animation.cancel();
    if (this.root) this.clear(this.root);
  }
}
