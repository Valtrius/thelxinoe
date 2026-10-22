import { api } from '../api';
import { trackPageWork } from '../page-readiness';
import { captureSession } from '../session';

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
