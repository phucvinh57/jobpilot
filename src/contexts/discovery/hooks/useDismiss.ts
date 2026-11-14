// "Dismiss" on the open lead: the lead leaves the list, with an optional reason; "Don't show this
// company again" also keeps it out of later searches.

import { useAction } from "@/shared";

import * as api from "../api/discoveryApi";
import type { Dismissed } from "../domain/types";
import { afterDismissed } from "./todayStore";

/** Dismiss lead `leadId`. `run(reason, excludeCompany)` resolves to what went, or `undefined` when it failed. */
export function useDismiss(leadId: number) {
  return useAction(async (reason: string, excludeCompany: boolean): Promise<Dismissed> => {
    const out = await api.dismissLead(leadId, reason, excludeCompany);
    afterDismissed(leadId, out);
    return out;
  });
}
