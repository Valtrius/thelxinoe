import { describe, expect, it } from 'vitest';
import { chapterAt, chapterMarks, chapterTitle } from './chapters';

const chapters = [
  { start: 0, title: 'Opening' },
  { start: 30, title: null },
  { start: 90, title: 'Credits' },
];

describe('chapters', () => {
  it('finds the chapter that has started and names untitled ones', () => {
    expect(chapterAt(chapters, 29.9)).toBe(0);
    expect(chapterAt(chapters, 30)).toBe(1);
    expect(chapterAt(chapters, 200)).toBe(2);
    expect(chapterAt([{ start: 10, title: null }], 5)).toBe(-1);
    expect(chapterTitle(chapters, 1)).toBe('Chapter 2');
  });

  it('marks chapter starts inside the media only', () => {
    expect(chapterMarks(chapters, 60)).toBe(
      'linear-gradient(to right, transparent 0 calc((var(--range-thumb) / 2 + (100% - var(--range-thumb)) * 0.5) - 1px), var(--background) 0 calc((var(--range-thumb) / 2 + (100% - var(--range-thumb)) * 0.5) + 1px), transparent 0)',
    );
    expect(chapterMarks(chapters.slice(0, 1), 60)).toBe('none');
  });
});
