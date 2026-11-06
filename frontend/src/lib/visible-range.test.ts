import { describe, expect, it } from 'vitest';
import { visibleRange } from './visible-range';

const list = { count: 5000, rowHeight: 77, viewportHeight: 770 };

describe('visibleRange', () => {
  it('renders the first rows before anything is measured', () => {
    expect(
      visibleRange({ ...list, scrollTop: 0, viewportHeight: 0 }),
    ).toStrictEqual({ start: 0, end: 6 });
  });

  it('renders the viewport and a margin around it, not the whole list', () => {
    expect(visibleRange({ ...list, scrollTop: 77 * 1000 })).toStrictEqual({
      start: 994,
      end: 1016,
    });
  });

  it('stays inside the list', () => {
    expect(visibleRange({ ...list, scrollTop: 77 * 5000 })).toStrictEqual({
      start: 4994,
      end: 5000,
    });
    expect(visibleRange({ ...list, count: 0, scrollTop: 0 })).toStrictEqual({
      start: 0,
      end: 0,
    });
    expect(visibleRange({ ...list, count: 3, scrollTop: -40 })).toStrictEqual({
      start: 0,
      end: 3,
    });
  });
});
