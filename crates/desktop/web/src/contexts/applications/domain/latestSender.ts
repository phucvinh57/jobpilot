// Send a stream of snapshots to the shell, one request at a time, and only ever the latest one:
// the answers typed in a review go out as full snapshots, so an older one that was never sent
// can simply be skipped.

export class LatestSender<T> {
  private pending: { value: T } | undefined;
  private running = false;
  private waiting: Array<() => void> = [];

  constructor(private readonly send: (value: T) => Promise<void>) {}

  /** Send `value`; if a send is running, it goes out when that one ends (replacing any value waiting). */
  push(value: T): void {
    this.pending = { value };
    if (!this.running) {
      void this.drain();
    }
  }

  /** Resolves when nothing is running and nothing is waiting. */
  idle(): Promise<void> {
    if (!this.running && this.pending === undefined) {
      return Promise.resolve();
    }
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  private async drain(): Promise<void> {
    this.running = true;
    try {
      while (this.pending !== undefined) {
        const { value } = this.pending;
        this.pending = undefined;
        try {
          await this.send(value);
        } catch (e) {
          console.warn("syncing the review failed", e);
        }
      }
    } finally {
      this.running = false;
      const waiting = this.waiting;
      this.waiting = [];
      for (const resolve of waiting) {
        resolve();
      }
    }
  }
}
