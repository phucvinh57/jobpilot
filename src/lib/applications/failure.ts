// A failure the shell keeps in what it returns (the last job's, the apply run's) comes back as a new
// object with every read. It is the same failure while its kind and message are the same.

import type { CommandError } from "@/lib/problem";

/** Whether `a` and `b` report the same failure (both none counts as the same). */
export function sameFailure(a: CommandError | null | undefined, b: CommandError | null | undefined): boolean {
  if (a == null || b == null) {
    return a == null && b == null;
  }
  return a.message === b.message && a.problem.kind === b.problem.kind;
}
