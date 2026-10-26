export type Chapter = { start: number; title: string | null };

/** The index of the chapter playing at a position, or -1 before the first. */
export function chapterAt(chapters: Chapter[], at: number) {
  return chapters.reduce(
    (current, chapter, index) => (chapter.start <= at ? index : current),
    -1,
  );
}

export function chapterTitle(chapters: Chapter[], index: number) {
  return chapters[index].title ?? `Chapter ${index + 1}`;
}

/** A `--range-marks` image cutting a gap into the seek bar where each chapter
 *  begins, at the thumb's centre for that time. */
export function chapterMarks(chapters: Chapter[], duration: number) {
  const gaps = chapters
    .filter((chapter) => chapter.start > 0 && chapter.start < duration)
    .map((chapter) => {
      const at = `(var(--range-thumb) / 2 + (100% - var(--range-thumb)) * ${chapter.start / duration})`;
      return `transparent 0 calc(${at} - 1px), var(--background) 0 calc(${at} + 1px)`;
    });
  return gaps.length
    ? `linear-gradient(to right, ${gaps.join(', ')}, transparent 0)`
    : 'none';
}
