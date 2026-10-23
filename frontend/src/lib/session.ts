let epoch = 0;

export function invalidateSession() {
  epoch++;
  navigationCache.clear();
}

export function captureSession(): () => boolean {
  const owner = epoch;
  return () => owner === epoch;
}
import { navigationCache } from './navigation-cache';
