import { describe, expect, it } from 'vitest';
import {
  selectedStatisticsSeconds,
  statisticsPlatformMetrics,
  statisticsDonut,
  gridNavigationTarget,
  completionDetails,
} from './helpers';

describe('statistics presentation', () => {
  it('includes local media in totals and uses consistent empty platform shares', () => {
    const sample = {
      youtubeSeconds: 10,
      twitchSeconds: 20,
      kickSeconds: 30,
      moviesSeconds: 40,
      showsSeconds: 50,
      musicSeconds: 60,
    };
    expect(selectedStatisticsSeconds(sample, 'all')).toBe(210);
    expect(selectedStatisticsSeconds(sample, 'music')).toBe(60);
    expect(selectedStatisticsSeconds(sample, 'shows')).toBe(50);
    const empty = statisticsPlatformMetrics(null, 'all');
    expect(statisticsDonut(empty)).toBe('var(--surface-soft)');
    expect(statisticsPlatformMetrics(null, 'movies').movies.share).toBe(100);
  });
  it('bounds arrow navigation and keeps Home and End in the same rhythm row', () => {
    expect(gridNavigationTarget(0, 'ArrowLeft', 7, 24)).toBe(0);
    expect(gridNavigationTarget(23, 'ArrowRight', 7, 24)).toBe(23);
    expect(gridNavigationTarget(25, 'Home', 7, 24)).toBe(24);
    expect(gridNavigationTarget(25, 'End', 7, 24)).toBe(47);
    expect(gridNavigationTarget(167, 'ArrowDown', 7, 24)).toBe(167);
    expect(completionDetails(null, 'music')).toMatchObject({
      started: 0,
      watched: 0,
      label: 'tracks',
    });
  });
});
