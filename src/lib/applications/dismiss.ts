// The words around "Dismiss" and "Restore" on a job in CVs.

import { ago } from "@/lib/display";

import type { Dismissal, Dismissed, Restored } from "@/types/applications";

/** What to tell the user after "Dismiss" on a job in CVs. */
export function jobDismissedNote(out: Dismissed): string {
  if (out.notExcluded !== null) {
    return `Job dismissed, but the company could not be excluded: ${out.notExcluded}`;
  }
  const more = (n: number) => (n === 1 ? "1 lead" : `${n} leads`);
  let text =
    out.also === 0
      ? "Job dismissed. Find it under Leads > Dismissed."
      : `Job dismissed, and ${more(out.also)} from the same company. Find them under Leads > Dismissed.`;
  if (out.kept > 0) {
    const verb = out.kept === 1 ? "was" : "were";
    text += ` ${more(out.kept)} from that company ${verb} kept: the CV is being prepared.`;
  }
  return text;
}

/** What to tell the user after "Restore" on a job from CVs, if anything: its company stays excluded. */
export function jobRestoredNote(out: Restored): string | undefined {
  return out.excluded
    ? "Job restored. Its company is still excluded in your preferences, so new searches leave it out."
    : undefined;
}

/** The line a dismissed job shows: `Dismissed 2 d ago: too far away`. */
export function dismissalLine(dismissal: Dismissal, now: Date): string {
  const then = dismissal.at === null ? Number.NaN : new Date(dismissal.at).getTime();
  const when = Number.isNaN(then) ? "Dismissed" : `Dismissed ${ago(now, new Date(then))}`;
  return dismissal.reason === null ? `${when}, no reason given.` : `${when}: ${dismissal.reason}`;
}
