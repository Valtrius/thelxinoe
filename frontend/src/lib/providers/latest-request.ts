/** Only the newest request may publish results, errors, or loading state. */
export class LatestRequest {
  private revision = 0;

  begin(): () => boolean {
    const revision = ++this.revision;
    const ownsSession = captureSession();
    return () => revision === this.revision && ownsSession();
  }

  invalidate() {
    this.revision += 1;
  }
}
import { captureSession } from '../session';
