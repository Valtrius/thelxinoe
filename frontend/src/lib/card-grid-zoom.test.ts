import { describe, expect, it } from 'vitest';
import {
  cardGridMetrics,
  nextCardColumns,
  youtubeMediaMetrics,
} from './card-grid-zoom';

describe('card grid metrics', () => {
  it('zooms three posters like two videos', () => {
    for (const [posters, videos] of [
      [9, 6],
      [6, 4],
      [18, 12],
    ]) {
      expect(cardGridMetrics(1600, posters, 'poster', 0)?.scale).toBeCloseTo(
        cardGridMetrics(1600, videos, 'video', 0)!.scale,
        10,
      );
    }
  });

  it('keeps a column just as wide whichever kind fills it', () => {
    for (const kind of ['poster', 'video'] as const) {
      const metrics = cardGridMetrics(1600, 8, kind)!;
      expect(metrics.logicalWidth * metrics.scale).toBeCloseTo(
        (1600 - 7 * 12) / 8,
        10,
      );
      expect(metrics.logicalGap * metrics.scale).toBeCloseTo(12, 10);
    }
  });

  it('sizes a sidebar alike beside three posters or two videos', () => {
    const posters = youtubeMediaMetrics(1600, 9, true, 'poster', 0, 0)!;
    const videos = youtubeMediaMetrics(1600, 6, true, 'video', 0, 0)!;
    expect(posters.sidebarWidth).toBeCloseTo(videos.sidebarWidth, 10);
    expect(posters.scale).toBeCloseTo(videos.scale, 10);
  });
});

describe('nextCardColumns', () => {
  it('stops at the range of each kind', () => {
    expect(nextCardColumns(5, -100, 'poster')).toBe(5);
    expect(nextCardColumns(18, 100, 'poster')).toBe(18);
    expect(nextCardColumns(3, -100, 'video')).toBe(3);
    expect(nextCardColumns(12, 100, 'video')).toBe(12);
  });

  it('steps by one column, whatever the wheel distance', () => {
    expect(nextCardColumns(9, 500, 'poster')).toBe(10);
    expect(nextCardColumns(9, -1, 'poster')).toBe(8);
    expect(nextCardColumns(9, 0, 'poster')).toBe(9);
  });

  it('moves a poster count saved below the range only up into it', () => {
    expect(nextCardColumns(3, 100, 'poster')).toBe(5);
    expect(nextCardColumns(4, 100, 'poster')).toBe(5);
    expect(nextCardColumns(3, -100, 'poster')).toBe(3);
    expect(nextCardColumns(4, -100, 'poster')).toBe(4);
  });
});
