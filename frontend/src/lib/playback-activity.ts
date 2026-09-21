/** Active wall time, independent of seek position and playback speed. */
export class PlaybackActivity {
  private last: number;
  private active = false;
  private total = 0;

  constructor(private now: () => number = () => performance.now()) {
    this.last = now();
  }

  setActive(active: boolean): boolean {
    this.sample();
    const changed = this.active !== active;
    this.active = active;
    return changed;
  }

  seconds(): number {
    this.sample();
    return this.total;
  }

  private sample(): void {
    const now = this.now();
    // Do not count an unbounded sleep/suspension between observations.
    if (this.active)
      this.total += Math.max(0, Math.min(30, (now - this.last) / 1000));
    this.last = now;
  }
}
