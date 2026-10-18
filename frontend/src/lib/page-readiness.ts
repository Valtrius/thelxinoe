import { tick } from 'svelte';

let active: PagePreparation | undefined;
let background = 0;

export function trackPageWork<T>(start: () => Promise<T>): Promise<T> {
  const finish = background ? undefined : active?.add();
  try {
    const pending = start();
    if (finish) void pending.then(finish, finish);
    return pending;
  } catch (error) {
    finish?.();
    throw error;
  }
}

// Content that shows its own loading state starts its work here, so the page
// appears without waiting for it. Only work started synchronously is detached.
export function inBackground<T>(start: () => T): T {
  background++;
  try {
    return start();
  } finally {
    background--;
  }
}

// Resolves once no page is waiting to appear.
export async function pageSettled() {
  while (active) await active.closed;
}

function untilAborted<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    void promise
      .then(resolve, reject)
      .finally(() => signal.removeEventListener('abort', abort));
  });
}

function pageFrame(signal: AbortSignal): Promise<void> {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const abort = () => {
      cancelAnimationFrame(frame);
      reject(signal.reason);
    };
    const frame = requestAnimationFrame(() => {
      signal.removeEventListener('abort', abort);
      resolve();
    });
    signal.addEventListener('abort', abort, { once: true });
  });
}

class PagePreparation {
  private pending = 0;
  private revision = 0;
  private finished = () => {};
  readonly closed = new Promise<void>((resolve) => (this.finished = resolve));

  add() {
    this.pending++;
    this.revision++;
    return () => {
      this.pending--;
      this.revision++;
    };
  }

  async ready(root: HTMLElement, viewport: HTMLElement, signal: AbortSignal) {
    await untilAborted(document.fonts.ready, signal);
    let stableFrames = 0;
    let revision = -1;
    let geometry = '';
    while (stableFrames < 2) {
      await tick();
      await pageFrame(signal);
      if (this.pending || revision !== this.revision) {
        revision = this.revision;
        stableFrames = 0;
        continue;
      }
      const bounds = viewport.getBoundingClientRect();
      const images = [
        ...root.querySelectorAll<HTMLImageElement>('img[src]'),
      ].filter((image) => {
        if (image.complete || !image.getClientRects().length) return false;
        const rect = image.getBoundingClientRect();
        return (
          rect.width > 0 &&
          rect.height > 0 &&
          rect.bottom >= bounds.top &&
          rect.top <= bounds.bottom &&
          rect.right >= bounds.left &&
          rect.left <= bounds.right
        );
      });
      if (images.length) {
        await untilAborted(
          Promise.allSettled(images.map((image) => image.decode())),
          signal,
        );
        stableFrames = 0;
        continue;
      }
      const next = `${root.clientWidth}:${root.clientHeight}:${viewport.scrollTop}:${root.scrollHeight}`;
      stableFrames = next === geometry ? stableFrames + 1 : 0;
      geometry = next;
    }
  }

  close() {
    if (active === this) active = undefined;
    this.finished();
  }
}

export function preparePage() {
  const preparation = new PagePreparation();
  active = preparation;
  return preparation;
}
