import { api } from '../api';
import { trackPageWork } from '../page-readiness';
import { captureSession } from '../session';
import type { Media, MediaKind } from './types';

export type SeerrStatus = { configured: boolean; ready?: boolean };

type Entry = { current: () => boolean; status: Promise<SeerrStatus> };
let cached: Entry | undefined;

// Asking costs a round trip to Seerr. A ready Seerr stays ready, so pages reuse
// that answer until the session ends, the stack changes or a Seerr call fails.
export function seerrStatus(): Promise<SeerrStatus> {
  let entry = cached;
  if (!entry?.current()) {
    const requested: Entry = {
      current: captureSession(),
      status: api<SeerrStatus>('/seerr/status'),
    };
    cached = entry = requested;
    void requested.status.then(
      (status) => {
        if (!(status.configured && status.ready) && cached === requested)
          cached = undefined;
      },
      () => {
        if (cached === requested) cached = undefined;
      },
    );
  }
  const { status } = entry;
  return trackPageWork(() => status);
}

export function forgetSeerrStatus() {
  cached = undefined;
}

const titles = new Map<string, { current: () => boolean; media: Media }>();

// A request's name and poster don't change, so the requests list reads each
// title once a session, unless the reader asks to read it again.
export async function seerrTitle(
  kind: MediaKind,
  id: number,
  reread = false,
): Promise<Media> {
  const key = `${kind}/${id}`;
  const cached = titles.get(key);
  if (cached?.current() && !reread) return cached.media;
  const current = captureSession();
  const media = await api<Media>(`/seerr/${key}`);
  if (current()) {
    titles.delete(key);
    titles.set(key, { current, media });
    if (titles.size > 200) titles.delete(titles.keys().next().value!);
  }
  return media;
}
