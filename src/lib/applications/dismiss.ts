// The words around "Dismiss" and "Restore" on a job in CVs.

import { ago } from "@/lib/display";

import type { Dismissal, Dismissed, Restored } from "@/types/applications";

/** What else "Dismiss" on a job in CVs did, under the "Job dismissed" toast. */
export function jobDismissedDetail(out: Pick<Dismissed, "also" | "kept">): string {
  const more = (n: number) => (n === 1 ? "1 lead" : `${n} leads`);
  const parts: string[] = [];
  if (out.also > 0) {
    parts.push(`${more(out.also)} from the same company went too.`);
  }
  if (out.kept > 0) {
    const verb = out.kept === 1 ? "was" : "were";
    parts.push(`${more(out.kept)} from that company ${verb} kept: the CV is being prepared.`);
  }
  parts.push("Find it under Leads > Dismissed.");
  return parts.join(" ");
}

/** What to say under the "Job restored" toast, if anything: its company stays excluded. */
export function jobRestoredDetail(out: Restored): string | undefined {
  return out.excluded ? "Its company is still excluded in your preferences, so new searches leave it out." : undefined;
}

/** The line a dismissed job shows: `Dismissed 2 d ago: too far away`. */
export function dismissalLine(dismissal: Dismissal, now: Date): string {
  const then = dismissal.at === null ? Number.NaN : new Date(dismissal.at).getTime();
  const when = Number.isNaN(then) ? "Dismissed" : `Dismissed ${ago(now, new Date(then))}`;
  return dismissal.reason === null ? `${when}, no reason given.` : `${when}: ${dismissal.reason}`;
}
