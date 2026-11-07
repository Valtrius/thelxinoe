import { describe, expect, it } from 'vitest';
import type { LayoutRect } from '../layout-animation';
import { flattenColors, rowChange } from './row-motion.svelte';

const viewport: LayoutRect = {
  top: 100,
  bottom: 700,
  left: 0,
  right: 1000,
  width: 1000,
  height: 600,
};
const at = (top: number, left = 0): LayoutRect => ({
  top,
  bottom: top + 30,
  left,
  right: left + 1000,
  width: 1000,
  height: 30,
});

describe('how a row changes', () => {
  it('leaves a row that stayed in place alone', () => {
    expect(rowChange(at(200), at(200), viewport)).toBe('none');
    expect(rowChange(at(200), at(200.4), viewport)).toBe('none');
  });

  it('moves a row between two places on screen', () => {
    expect(rowChange(at(200), at(400), viewport)).toBe('move');
    expect(rowChange(at(200), at(200, 8), viewport)).toBe('move');
  });

  it('moves a row off screen when the distance is short', () => {
    expect(rowChange(at(600), at(900), viewport)).toBe('move');
  });

  it('fades in a row that was not on screen', () => {
    expect(rowChange(undefined, at(200), viewport)).toBe('arrive');
    expect(rowChange(at(4000), at(200), viewport)).toBe('arrive');
  });

  it('ignores a row that stays off screen', () => {
    expect(rowChange(undefined, at(4000), viewport)).toBe('none');
    expect(rowChange(at(5000), at(4000), viewport)).toBe('none');
  });

  it('never sends a row across more than a screen', () => {
    expect(rowChange(at(700), at(75), viewport)).toBe('jump');
    expect(rowChange(at(200), at(4000), viewport)).toBe('leave');
  });
});

describe('flattening backgrounds', () => {
  it('composites translucent layers over the first opaque one', () => {
    expect(
      flattenColors([
        'rgba(0, 0, 0, 0)',
        'rgba(13, 18, 24, 0.86)',
        'rgb(5, 7, 11)',
      ]),
    ).toBe('rgb(12 16 22)');
  });

  it('ignores anything beneath an opaque layer', () => {
    expect(flattenColors(['rgb(1, 2, 3)', 'rgb(200, 200, 200)'])).toBe(
      'rgb(1 2 3)',
    );
  });

  it('skips formats it cannot composite', () => {
    expect(
      flattenColors(['color(srgb 0.1 0.2 0.3 / 0.5)', 'rgb(4, 5, 6)']),
    ).toBe('rgb(4 5 6)');
  });

  it('falls back to white when nothing is opaque', () => {
    expect(flattenColors(['rgba(0, 0, 0, 0)'])).toBe('rgb(255 255 255)');
  });
});
