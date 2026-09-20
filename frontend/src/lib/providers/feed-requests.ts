import { LatestRequest } from './latest-request';

/** Coordinates replacement, pagination, and refresh of one retained feed. */
export class FeedRequests extends LatestRequest {
  private pendingLoad: Promise<void> | null = null;

  load(run: (current: () => boolean) => Promise<void>): Promise<void> {
    const request = run(this.begin());
    this.pendingLoad = request;
    return request.finally(() => {
      if (this.pendingLoad === request) this.pendingLoad = null;
    });
  }

  async finishLoading() {
    while (this.pendingLoad) await this.pendingLoad;
  }
}
