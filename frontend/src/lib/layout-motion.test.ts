import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  feedLayoutMotionOptions,
  createLayoutMotion,
  layoutTransform,
  type LayoutRect,
} from './layout-motion';

function rect(
  left: number,
  top: number,
  width: number,
  height: number,
): LayoutRect {
  return {
    top,
    right: left + width,
    bottom: top + height,
    left,
    width,
    height,
  };
}

describe('layout motion geometry', () => {
  it('projects movement and width changes without scaling height', () => {
    expect(
      layoutTransform(
        rect(20, 40, 240, 300),
        rect(32, 80, 300, 420),
        feedLayoutMotionOptions,
      ),
    ).toEqual({ x: -12, y: -40, scaleX: 0.8, scaleY: 1 });
  });

  it('skips changes below the configured tolerances', () => {
    expect(
      layoutTransform(
        rect(20, 40, 240, 300),
        rect(20.2, 39.8, 240.4, 360),
        feedLayoutMotionOptions,
      ),
    ).toBeNull();
  });

  it('keeps height-only card-grid changes animatable', () => {
    expect(
      layoutTransform(
        rect(20, 40, 240, 300),
        rect(20, 40, 240, 400),
        feedLayoutMotionOptions,
        true,
      ),
    ).toEqual({ x: 0, y: 0, scaleX: 1, scaleY: 0.75 });
  });
});

describe('layout motion lifecycle', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('reads each grid scale once per pass and does no layout work on completion', async () => {
    let zoom = '0.5';
    const parent = {
      hasAttribute: (name: string) => name === 'data-card-grid',
    };
    const cancel = vi.fn();
    const items = Array.from({ length: 120 }, (_, index) => ({
      dataset: { layoutKey: String(index) },
      parentElement: parent,
      getBoundingClientRect: vi.fn(() =>
        rect((index % 6) * 160, Math.floor(index / 6) * 100, 160, 100),
      ),
      animate: vi.fn(() => ({ cancel, finished: Promise.resolve() })),
    }));
    const root = {
      getBoundingClientRect: vi.fn(() => rect(0, 0, 960, 500)),
      querySelectorAll: () => items,
      scrollTop: 0,
      scrollLeft: 0,
    };
    const computedStyle = vi.fn(() => ({ zoom, getPropertyValue: () => '' }));
    vi.stubGlobal('getComputedStyle', computedStyle);
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    const motion = createLayoutMotion();
    const connection = motion.connect(root as unknown as HTMLElement);
    const snapshot = motion.capture();
    expect(computedStyle).toHaveBeenCalledTimes(1);
    expect(snapshot?.items.get('0')?.cardGridZoom).toBe(0.5);

    zoom = '0.6';
    for (const item of items) {
      item.getBoundingClientRect.mockImplementation(() => rect(0, 0, 192, 120));
    }
    motion.play(snapshot);
    expect(computedStyle).toHaveBeenCalledTimes(3);
    expect(
      items.every((item) => item.getBoundingClientRect.mock.calls.length === 2),
    ).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
    expect(computedStyle).toHaveBeenCalledTimes(3);
    expect(
      items.every((item) => item.getBoundingClientRect.mock.calls.length === 2),
    ).toBe(true);
    connection.destroy();
  });

  it('captures an interrupted animation before canceling it', () => {
    let current = rect(10, 20, 160, 100);
    const cancel = vi.fn(() => {
      current = rect(0, 0, 320, 200);
    });
    const element = {
      dataset: { layoutKey: 'video' },
      parentElement: { hasAttribute: () => true },
      getBoundingClientRect: () => current,
      animate: () => ({ cancel, finished: new Promise<void>(() => {}) }),
    };
    const root = {
      getBoundingClientRect: () => rect(0, 0, 960, 500),
      querySelectorAll: () => [element],
      scrollTop: 0,
      scrollLeft: 0,
    };
    vi.stubGlobal('getComputedStyle', () => ({
      zoom: '1',
      getPropertyValue: () => '',
    }));
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    const motion = createLayoutMotion();
    const connection = motion.connect(root as unknown as HTMLElement);
    const first = motion.capture();
    current = rect(0, 0, 320, 200);
    motion.play(first);
    current = rect(5, 10, 240, 150);
    const interrupted = motion.capture();
    expect(interrupted?.items.get('video')?.rect).toEqual(
      rect(5, 10, 240, 150),
    );
    expect(cancel).toHaveBeenCalledOnce();
    connection.destroy();
  });
});
