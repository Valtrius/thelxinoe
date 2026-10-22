import { describe, expect, it } from 'vitest';
import { backdropSet, posterSet } from './types';

describe('TMDB artwork', () => {
  it('lists the widths TMDB renders for the browser to choose from', () => {
    expect(posterSet('/poster.jpg')).toBe(
      [92, 154, 185, 342, 500]
        .map(
          (width) =>
            `https://image.tmdb.org/t/p/w${width}/poster.jpg ${width}w`,
        )
        .join(),
    );
    expect(backdropSet('/backdrop.jpg')).toBe(
      [300, 780, 1280]
        .map(
          (width) =>
            `https://image.tmdb.org/t/p/w${width}/backdrop.jpg ${width}w`,
        )
        .join(),
    );
    expect(posterSet('https://example.com/poster.jpg')).toBeUndefined();
    expect(backdropSet(null)).toBeUndefined();
  });
});
