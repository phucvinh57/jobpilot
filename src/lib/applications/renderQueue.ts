// The CV editor saves and re-renders after a quiet moment, one render at a time.

/** Quiet time after the last keystroke before the CV is saved and re-rendered (ms). */
export const EDIT_DEBOUNCE = 800;

/** Latest-wins debounce: `touch()` on every edit, then after the quiet time act only if
 *  `isLatest(token)` still holds. */
export class Debounce {
  private generation = 0;

  touch(): number {
    this.generation += 1;
    return this.generation;
  }

  isLatest(token: number): boolean {
    return token === this.generation;
  }

  /** Invalidate every pending token (e.g. the text was replaced from disk). */
  cancel(): void {
    this.generation += 1;
  }
}

/** One render at a time: edits that arrive during a render cause exactly one more render afterwards. */
export class RenderQueue {
  private running = false;
  private again = false;

  /** A render is wanted. `true` = start it now; `false` = one is running and will be repeated. */
  request(): boolean {
    if (this.running) {
      this.again = true;
      return false;
    }
    this.running = true;
    return true;
  }

  /** The running render ended. `true` = another one is due (start it now). */
  finish(): boolean {
    if (this.again) {
      this.again = false;
      return true;
    }
    this.running = false;
    return false;
  }

  isRunning(): boolean {
    return this.running;
  }

  /** Forget everything (the application changed). */
  reset(): void {
    this.running = false;
    this.again = false;
  }
}
