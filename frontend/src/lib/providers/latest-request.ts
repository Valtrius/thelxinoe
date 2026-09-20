/** Only the newest request may publish results, errors, or loading state. */
export class LatestRequest {
  private revision = 0;

  begin(): () => boolean {
    const revision = ++this.revision;
    return () => revision === this.revision;
  }

  invalidate() {
    this.revision += 1;
  }
}
