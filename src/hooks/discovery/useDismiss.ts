// "Dismiss" on the open lead: the lead leaves the list, with an optional reason; "Don't show this
// company again" also keeps it out of later searches.

import { useAction } from "@/hooks/useAsync";

import * as api from "@/api/discovery";
import type { Dismissed } from "@/types/discovery";
import { afterDismissed } from "@/stores/today";

/** Dismiss lead `leadId`. `run(reason, excludeCompany)` resolves to what went, or `undefined` when it failed. */
export function useDismiss(leadId: number) {
  return useAction(async (reason: string, excludeCompany: boolean): Promise<Dismissed> => {
    const out = await api.dismissLead(leadId, reason, excludeCompany);
    afterDismissed(leadId, out);
    return out;
  });
}
