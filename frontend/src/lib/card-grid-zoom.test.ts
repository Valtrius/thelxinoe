import { describe, expect, it } from 'vitest';
import {
  cardGridMetrics,
  cardGridTemplate,
  CARD_LOGICAL_WIDTH_PX,
  DEFAULT_CARD_COLUMNS,
  nextCardColumns,
  parseCardColumns,
  youtubeMediaMetrics,
} from './card-grid-zoom';

describe('card grid zoom', () => {
  it('restores only valid saved column counts', () => {
    expect(parseCardColumns('8')).toBe(8);
    expect(parseCardColumns('3')).toBe(3);
    expect(parseCardColumns('12')).toBe(12);
    expect(parseCardColumns(null)).toBeNull();
    expect(parseCardColumns('2')).toBeNull();
    expect(parseCardColumns('13')).toBeNull();
    expect(parseCardColumns('8.5')).toBeNull();
    expect(parseCardColumns('nope')).toBeNull();
  });

  it('moves one column per wheel event and clamps the range', () => {
    expect(nextCardColumns(6, 100)).toBe(7);
    expect(nextCardColumns(6, -100)).toBe(5);
    expect(nextCardColumns(6, 0)).toBe(6);
    expect(nextCardColumns(3, -100)).toBe(3);
    expect(nextCardColumns(12, 100)).toBe(12);
  });

  it('uses six columns by default', () => {
    expect(DEFAULT_CARD_COLUMNS).toBe(6);
    expect(cardGridTemplate(DEFAULT_CARD_COLUMNS)).toBe('repeat(6,320px)');
    expect(cardGridTemplate(7)).toBe('repeat(7,320px)');
  });

  it('keeps the logical card width fixed while the rendered card scales', () => {
    const zoomedIn = cardGridMetrics(1200, 3, 12);
    const normal = cardGridMetrics(1200, 6, 12);
    const zoomedOut = cardGridMetrics(1200, 12, 12);

    expect(CARD_LOGICAL_WIDTH_PX).toBe(320);
    expect(normal?.logicalWidth).toBe(320);
    expect(normal?.scale).toBeCloseTo(190 / 320);
    expect(normal?.logicalGap && normal.logicalGap * normal.scale).toBeCloseTo(
      12,
    );
    expect(zoomedIn?.logicalWidth).toBe(320);
    expect(
      zoomedIn?.logicalWidth && zoomedIn.scale * zoomedIn.logicalWidth,
    ).toBeCloseTo(392);
    expect(zoomedOut?.logicalWidth).toBe(320);
    expect(
      zoomedOut?.logicalWidth && zoomedOut.scale * zoomedOut.logicalWidth,
    ).toBeCloseTo(89);
  });

  it('keeps the logical width stable when sidebars resize the grid', () => {
    const expandedSidebar = cardGridMetrics(900, 6, 12);
    const collapsedSidebar = cardGridMetrics(1080, 6, 12);

    expect(expandedSidebar?.logicalWidth).toBe(320);
    expect(collapsedSidebar?.logicalWidth).toBe(320);
    expect(expandedSidebar?.scale).toBeCloseTo(140 / 320);
    expect(collapsedSidebar?.scale).toBeCloseTo(170 / 320);
  });

  it('uses one shared scale for YouTube cards and the watchlist sidebar', () => {
    const metrics = youtubeMediaMetrics(1800, 6, true, 16);
    expect(metrics).not.toBeNull();
    if (!metrics) return;
    expect(metrics.sidebarWidth).toBeCloseTo(430 * metrics.scale);
    expect(
      6 * 320 * metrics.scale + metrics.sidebarWidth + 5 * 12 + 16,
    ).toBeCloseTo(1800);
  });
});
