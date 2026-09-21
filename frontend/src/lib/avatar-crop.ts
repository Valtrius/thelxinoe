export function cropRectangle(
  width: number,
  height: number,
  zoom: number,
  horizontal: number,
  vertical: number,
) {
  const size = Math.min(width, height) / Math.max(1, Math.min(4, zoom));
  return {
    x: Math.max(0, Math.min(1, horizontal)) * (width - size),
    y: Math.max(0, Math.min(1, vertical)) * (height - size),
    size,
  };
}
