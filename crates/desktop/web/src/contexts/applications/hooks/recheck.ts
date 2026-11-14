// A failure that names a missing prerequisite (no provider, no Typst, no Chrome) makes the shell read
// the setup state again, so the health line and the banner follow what the user just learned.

import { needsRecheck, toCommandError, useShell } from "@/shared";

/** Run `run`; when it fails because a prerequisite is missing, ask the shell to check again. The failure
 *  is rethrown as a `CommandError`. */
export async function recheckOnFailure<T>(run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (thrown) {
    const error = toCommandError(thrown);
    if (needsRecheck(error.problem)) {
      useShell.getState().refresh();
    }
    throw error;
  }
}
