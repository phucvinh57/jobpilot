// "Dismiss" and "Restore" on one job in CVs, from its drawer or its full view. A dismissed job keeps
// its application: Leads > Dismissed lists it, and Restore puts it back at the status it had.

import { useAction, type Action } from "@/hooks/useAsync";

import { dismissApplication, restoreApplication } from "@/api/applications";
import type { Dismissed, Restored } from "@/types/applications";

export interface DismissJob {
  /** `run(reason, excludeCompany)` resolves to what went, or `undefined` when it failed. */
  dismiss: Action<[reason: string, excludeCompany: boolean], Dismissed>;
  /** Resolves to what it did (the company still excluded), or `undefined` when it failed. */
  restore: Action<[], Restored>;
}

/** Used inside content keyed by the application, so a failure never carries over to another one. */
export function useDismissJob(id: number): DismissJob {
  return {
    dismiss: useAction((reason: string, excludeCompany: boolean) => dismissApplication(id, reason, excludeCompany)),
    restore: useAction(() => restoreApplication(id)),
  };
}
