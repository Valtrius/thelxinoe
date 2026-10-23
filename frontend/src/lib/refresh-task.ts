/** One visible-page read at a time, with one catch-up for changes during it. */
export class RefreshTask {
  private running?: Promise<void>;
  private dirty = false;
  private stopped = false;
  private started = false;
  private completed = Date.now();
  private timer?: ReturnType<typeof setTimeout>;

  constructor(
    private load: () => Promise<void>,
    private interval: number | (() => number),
    private pollWhen: () => boolean = () => true,
  ) {}

  refresh = (): Promise<void> => {
    if (this.stopped) return Promise.resolve();
    this.dirty = true;
    clearTimeout(this.timer);
    this.timer = undefined;
    if (document.hidden) return Promise.resolve();
    return (this.running ??= this.drain().finally(() => {
      this.running = undefined;
      this.completed = Date.now();
      if (this.dirty && !document.hidden && !this.stopped)
        return this.refresh();
      this.schedule();
    }));
  };

  private async drain() {
    do {
      this.dirty = false;
      await this.load();
    } while (this.dirty && !document.hidden && !this.stopped);
  }

  private delay() {
    return typeof this.interval === 'number' ? this.interval : this.interval();
  }

  private schedule() {
    clearTimeout(this.timer);
    this.timer = undefined;
    if (!this.started || this.stopped || document.hidden || this.running)
      return;
    this.timer = setTimeout(this.poll, this.delay());
  }

  private poll = () => {
    if (this.running || document.hidden || this.stopped) return;
    if (this.dirty || this.pollWhen()) void this.refresh().catch(() => {});
    else this.schedule();
  };

  private resume = () => {
    if (document.hidden) {
      clearTimeout(this.timer);
      this.timer = undefined;
      return;
    }
    if (this.running || this.stopped) return;
    if (this.dirty || Date.now() - this.completed >= this.delay())
      void this.refresh().catch(() => {});
    else if (!this.timer) this.schedule();
  };

  private online = () => {
    if (!this.running) void this.refresh().catch(() => {});
  };

  start(initial = true) {
    this.started = true;
    document.addEventListener('visibilitychange', this.resume);
    window.addEventListener('focus', this.resume);
    window.addEventListener('online', this.online);
    if (initial) void this.refresh().catch(() => {});
    else this.schedule();
    return this.stop;
  }

  stop = () => {
    this.stopped = true;
    this.dirty = false;
    clearTimeout(this.timer);
    document.removeEventListener('visibilitychange', this.resume);
    window.removeEventListener('focus', this.resume);
    window.removeEventListener('online', this.online);
  };
}
