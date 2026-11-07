import { formatClock } from './providers/utils';

export type AnalysisStatus =
  'found' | 'nothing' | 'failed' | 'waiting' | 'running';
export type AnalysisSegment = { kind: string; start: number; end: number };
export type AnalyzedEpisode = {
  media_id: string;
  file_id: string;
  title: string;
  status: AnalysisStatus;
  error: string | null;
  analyzed_at: number | null;
  segments: AnalysisSegment[];
};
export type AnalysisCoverage = {
  episodes: number;
  intro: number;
  credits: number;
  nothing: number;
  failed: number;
  waiting: number;
};
export type AnalysisShow = AnalysisCoverage & { id: string; title: string };
export type AnalysisPage = {
  items: AnalyzedEpisode[];
  shows: AnalysisShow[];
  totals: AnalysisCoverage;
};
export type AnalysisFilters = {
  show: string;
  status: '' | 'nothing' | 'failed' | 'waiting';
};

export const analysisPageSize = 100;

const range = (segment: AnalysisSegment) =>
  `${formatClock(segment.start)}–${formatClock(segment.end)}`;

/** When a kind plays, one entry for each time it does. */
export function segmentTimes(
  segments: AnalysisSegment[],
  kind: string,
): string[] {
  return segments.filter((segment) => segment.kind === kind).map(range);
}

/** The kinds without a column of their own, such as a recap from TheIntroDB. */
export function otherSegments(segments: AnalysisSegment[]): string {
  return segments
    .filter((segment) => !['Intro', 'Credits'].includes(segment.kind))
    .map((segment) => `${segment.kind} ${range(segment)}`)
    .join(' · ');
}

const count = (value: number, one: string, many = `${one}s`) =>
  `${value} ${value === 1 ? one : many}`;

export function coverageSummary(totals: AnalysisCoverage): string {
  return [
    count(totals.episodes, 'episode'),
    count(totals.intro, 'intro'),
    count(totals.credits, 'credit'),
    totals.nothing && `${totals.nothing} nothing found`,
    totals.failed && `${totals.failed} failed`,
    totals.waiting && `${totals.waiting} waiting`,
  ]
    .filter(Boolean)
    .join(' · ');
}

export function analysisQuery(
  filters: AnalysisFilters,
  limit?: number,
): string {
  const query = new URLSearchParams();
  if (filters.show) query.set('show', filters.show);
  if (filters.status) query.set('status', filters.status);
  if (limit) query.set('limit', String(limit));
  return query.toString();
}

/** The coverage the filters look at: one show's, or the whole library's. */
export function filteredCoverage(
  page: Pick<AnalysisPage, 'shows' | 'totals'>,
  show: string,
): AnalysisCoverage | undefined {
  return show ? page.shows.find((item) => item.id === show) : page.totals;
}

/** Whether more episodes match than the ones listed. */
export function hasMoreEpisodes(
  page: AnalysisPage,
  filters: AnalysisFilters,
): boolean {
  const coverage = filteredCoverage(page, filters.show);
  if (!coverage) return false;
  const matching = filters.status
    ? coverage[filters.status]
    : coverage.episodes;
  return page.items.length < matching;
}
