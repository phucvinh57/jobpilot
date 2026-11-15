// "Dismiss" and "Restore" on one job in CVs, from its drawer or its full view. A dismissed job keeps
// its application: Leads > Dismissed lists it, and Restore puts it back at the status it had. A toast
// says what went (or that its company could not be excluded) and where a restored job went.

import { useAction, type Action } from "@/hooks/useAsync";

import { dismissApplication, restoreApplication } from "@/api/applications";
import { jobDismissedDetail, jobRestoredDetail } from "@/lib/applications/dismiss";
import { CommandError } from "@/lib/problem";
import type { Dismissed, Restored } from "@/types/applications";
import { toast } from "@/stores/toast";

export interface DismissJob {
  /** `run(reason, excludeCompany)` resolves to what went, or `undefined` when it failed (the
   *  failure stays in `error`, for the dialog to show). */
  dismiss: Action<[reason: string, excludeCompany: boolean], Dismissed>;
  /** Resolves to what it did, or `undefined` when it failed (the failure is in `error`). */
  restore: Action<[], Restored>;
}

/** Used inside content keyed by the application, so a failure never carries over to another one. */
export function useDismissJob(id: number): DismissJob {
  return {
    dismiss: useAction(async (reason: string, excludeCompany: boolean) => {
      const out = await dismissApplication(id, reason, excludeCompany);
      if (out.notExcluded !== null) {
        toast.error(new CommandError(out.notExcluded), {
          title: "Job dismissed, but the company could not be excluded",
        });
      } else {
        toast.ok("Job dismissed", jobDismissedDetail(out));
      }
      return out;
    }),
    restore: useAction(async () => {
      const out = await restoreApplication(id);
      toast.ok("Job restored", jobRestoredDetail(out));
      return out;
    }),
  };
}
