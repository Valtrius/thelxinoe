import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  captureSidebarResize,
  sidebarResizePlans,
  sidebarResizeTransform,
} from './sidebar-motion';
import type { LayoutRect } from './layout-animation';

function rect(top: number, height: number): LayoutRect {
  return { top, height, bottom: top + height, left: 0, right: 200, width: 200 };
}

function bounds(
  left: number,
  top: number,
  width: number,
  height: number,
): LayoutRect {
  return {
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
  };
}

function frameTransform(frame: Keyframe): number[] {
  return String(frame.transform)
    .match(/[-+]?(?:\d*\.?\d+)(?:e[-+]?\d+)?/g)!
    .map(Number);
}

describe('sections inside an animated scroll viewport', () => {
  afterEach(() => vi.unstubAllGlobals());

  it.each(['collapse', 'expand'])(
    'preserves control sizes and section geometry during %s',
    (direction) => {
      vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
      const rectangles = [
        [bounds(180, 32, 1080, 700), bounds(72, 32, 1188, 700)],
        // Right-aligned filters stay in place as the viewport changes width.
        [bounds(1000, 44, 228, 32), bounds(1000, 44, 228, 32)],
        // A chart changes both size and its position inside a two-column row.
        [bounds(600, 400, 600, 300), bounds(550, 400, 650, 350)],
        // The following section moves down as that chart grows.
        [bounds(192, 712, 1056, 300), bounds(84, 762, 1164, 280)],
      ].map((pair) => (direction === 'collapse' ? pair : [...pair].reverse()));
      let phase = 0;
      const origin = {
        dataset: { sidebarResize: 'x' },
        parentElement: { closest: () => null },
        getBoundingClientRect: () => rectangles[0][phase],
        getAnimations: () => [],
        hasAttribute: () => false,
      };
      const sections = rectangles.slice(1).map((pair) => ({
        dataset: { sidebarResize: 'xy' },
        parentElement: { closest: () => origin, scrollTop: 0 },
        getBoundingClientRect: () => pair[phase],
        getAnimations: () => [],
        hasAttribute: () => false,
      }));
      const root = { querySelectorAll: () => [origin, ...sections] };
      const snapshots = captureSidebarResize(root as unknown as HTMLElement);
      phase = 1;
      const plans = sidebarResizePlans(snapshots);
      const originTransform = sidebarResizeTransform(
        rectangles[0][0],
        rectangles[0][1],
        'x',
        1,
        0,
        0,
      );
      const finalOrigin = rectangles[0][1];

      for (const [index, section] of sections.entries()) {
        const plan = plans.find(
          (plan) => plan.element === (section as unknown as HTMLElement),
        )!;
        expect(plan).toBeDefined();
        const [before, final] = rectangles[index + 1];
        for (
          let frameIndex = 0;
          frameIndex < plan.keyframes.length - 1;
          frameIndex++
        ) {
          const start = plan.keyframes[frameIndex];
          const end = plan.keyframes[frameIndex + 1];
          const startTransform = frameTransform(start);
          const endTransform = frameTransform(end);
          // Check browser interpolation between the sampled compensation frames too.
          for (const fraction of [0, 0.5, 1]) {
            const progress =
              Number(start.offset) +
              (Number(end.offset) - Number(start.offset)) * fraction;
            const [x, y, scaleX, scaleY] = startTransform.map(
              (value, axis) => value + (endTransform[axis] - value) * fraction,
            );
            const parentScaleX =
              1 + (originTransform.scaleX - 1) * (1 - progress);
            const actual = [
              finalOrigin.left +
                originTransform.x * (1 - progress) +
                parentScaleX * (final.left - finalOrigin.left + x),
              final.top + y,
              final.width * parentScaleX * scaleX,
              final.height * scaleY,
            ];
            const expected = ['left', 'top', 'width', 'height'].map((key) => {
              const axis = key as keyof LayoutRect;
              return before[axis] + (final[axis] - before[axis]) * progress;
            });
            actual.forEach((value, axis) =>
              expect(Math.abs(value - expected[axis])).toBeLessThan(0.15),
            );
          }
        }
      }
    },
  );
});

describe('watchlist resize geometry', () => {
  it('keeps toolbar height at its previous size when the viewport height stays fixed', () => {
    const before = rect(0, 60 * 0.4);
    const after = rect(0, 60 * 0.5);
    const transform = sidebarResizeTransform(before, after, 'y', 0.5, 0, 0);
    expect(after.height * transform.scaleY).toBeCloseTo(before.height);
    expect(transform.scaleX).toBe(1);
  });

  it('preserves a scrolled row position when zoom and scroll offset both change', () => {
    const oldZoom = 0.4;
    const newZoom = 0.5;
    const oldScroll = 300;
    const newScroll = 280;
    const rowTop = 400;
    const before = rect(-oldScroll * oldZoom, 2000 * oldZoom);
    const after = rect(-newScroll * newZoom, 2000 * newZoom);
    const transform = sidebarResizeTransform(
      before,
      after,
      'y-scale-scroll',
      newZoom,
      oldScroll,
      newScroll,
    );
    const animatedTop =
      (-newScroll + transform.y + rowTop * transform.scaleY) * newZoom;
    expect(animatedTop).toBeCloseTo((rowTop - oldScroll) * oldZoom);
    expect(80 * newZoom * transform.scaleY).toBeCloseTo(80 * oldZoom);
  });
});
