let epoch = 0;

export function invalidateSession() {
  epoch++;
}

export function captureSession(): () => boolean {
  const owner = epoch;
  return () => owner === epoch;
}
