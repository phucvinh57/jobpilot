// What "Dismiss" and "Restore" did, as both the Leads and the CVs side report it (the shell's
// shared `DismissedDto` / `RestoredDto`).

/** What "Dismiss" did. */
export interface Dismissed {
  /** Other leads of the company dismissed with it ("Don't show {company} again"). */
  also: number;
  /** Other leads of the company kept because their CV is queued or being written. */
  kept: number;
  /** "Don't show {company} again" was asked for but could not be saved (why). */
  notExcluded: string | null;
}

/** What "Restore" did. */
export interface Restored {
  /** The restored lead is hidden by the saved preferences (why): back, but not in the list until
   *  they let it pass. `null` when it is shown, and for a job from CVs. */
  hidden: string | null;
  /** Its company is still excluded in the preferences: later searches keep leaving it out. */
  excluded: boolean;
}
