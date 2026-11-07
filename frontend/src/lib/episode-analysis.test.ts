import { describe, expect, it } from 'vitest';
import {
  analysisQuery,
  coverageSummary,
  filteredCoverage,
  hasMoreEpisodes,
  otherSegments,
  segmentTimes,
  type AnalysisCoverage,
  type AnalysisPage,
  type AnalyzedEpisode,
} from './episode-analysis';

const coverage = (
  fields: Partial<AnalysisCoverage> = {},
): AnalysisCoverage => ({
  episodes: 0,
  intro: 0,
  credits: 0,
  nothing: 0,
  failed: 0,
  waiting: 0,
  ...fields,
});
const episode = (media_id: string): AnalyzedEpisode => ({
  media_id,
  file_id: `${media_id}-file`,
  title: media_id,
  status: 'found',
  error: null,
  analyzed_at: 1,
  segments: [],
});

describe('segment times', () => {
  const segments = [
    { kind: 'Recap', start: 0, end: 62.4 },
    { kind: 'Intro', start: 127, end: 188.9 },
    { kind: 'Credits', start: 2410, end: 3700 },
    { kind: 'Credits', start: 3800, end: 3900 },
  ];

  it('formats each kind, with hours once they are needed', () => {
    expect(segmentTimes(segments, 'Intro')).toEqual(['2:07–3:08']);
    expect(segmentTimes(segments, 'Credits')).toEqual([
      '40:10–1:01:40',
      '1:03:20–1:05:00',
    ]);
    expect(segmentTimes(segments, 'Preview')).toEqual([]);
  });

  it('names the kinds without a column', () => {
    expect(otherSegments(segments)).toBe('Recap 0:00–1:02');
    expect(otherSegments(segments.slice(1))).toBe('');
  });
});

describe('coverage summary', () => {
  it('counts everything found and leaves out empty problems', () => {
    expect(
      coverageSummary(coverage({ episodes: 1, intro: 1, credits: 1 })),
    ).toBe('1 episode · 1 intro · 1 credit');
    expect(
      coverageSummary(
        coverage({
          episodes: 637,
          intro: 564,
          credits: 321,
          nothing: 46,
          failed: 1,
          waiting: 3,
        }),
      ),
    ).toBe(
      '637 episodes · 564 intros · 321 credits · 46 nothing found · 1 failed · 3 waiting',
    );
  });
});

describe('analysis queries', () => {
  it('sends only the filters in use', () => {
    expect(analysisQuery({ show: '', status: '' })).toBe('');
    expect(analysisQuery({ show: 'a b', status: 'failed' }, 200)).toBe(
      'show=a+b&status=failed&limit=200',
    );
  });
});

describe('more episodes', () => {
  const show = {
    id: 'show',
    title: 'Show',
    ...coverage({ episodes: 3, nothing: 1 }),
  };
  const page = (count: number): AnalysisPage => ({
    items: Array.from({ length: count }, (_, index) => episode(`e${index}`)),
    shows: [show],
    totals: coverage({ episodes: 5, nothing: 2 }),
  });

  it('compares the listed episodes with everything matching', () => {
    expect(hasMoreEpisodes(page(4), { show: '', status: '' })).toBe(true);
    expect(hasMoreEpisodes(page(5), { show: '', status: '' })).toBe(false);
  });

  it('narrows to the status and show', () => {
    expect(hasMoreEpisodes(page(1), { show: '', status: 'nothing' })).toBe(
      true,
    );
    expect(hasMoreEpisodes(page(2), { show: '', status: 'nothing' })).toBe(
      false,
    );
    expect(hasMoreEpisodes(page(2), { show: 'show', status: '' })).toBe(true);
    expect(hasMoreEpisodes(page(1), { show: 'show', status: 'nothing' })).toBe(
      false,
    );
  });

  it('stops once the chosen show is gone', () => {
    expect(hasMoreEpisodes(page(0), { show: 'gone', status: '' })).toBe(false);
    expect(filteredCoverage(page(0), 'gone')).toBeUndefined();
  });
});
