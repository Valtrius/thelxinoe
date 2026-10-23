export function libraryArtworkSet(value: string): string | undefined {
  if (!value) return undefined;
  try {
    const url = new URL(value, 'https://thelxinoe.invalid');
    if (
      !/^\/api\/v1\/catalog\/(?:[^/]+\/artwork|titles\/[^/]+\/\d+\/artwork\/poster)$/.test(
        url.pathname,
      )
    )
      return undefined;
    return [500, 1000]
      .map((width) => {
        url.searchParams.set('width', String(width));
        return `${value.startsWith('/') ? url.pathname + url.search : url.href} ${width}w`;
      })
      .join(', ');
  } catch {
    return undefined;
  }
}
