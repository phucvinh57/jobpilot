// The breadcrumb of the Application screen leads back to the Jobs page, at the stage (and with the
// drawer) the user left it.

import type { Stage } from "@/stores/nav";

import { stageLabel } from "./pipeline";

export interface Crumb {
  id: string;
  /** `Jobs › CVs` */
  label: string;
  aria: string;
}

/** The breadcrumb back to the Jobs page at `stage`. */
export function crumbFor(stage: Stage): Crumb {
  const name = stage === "leads" ? "Leads" : stageLabel(stage);
  return { id: "breadcrumb-jobs", label: `Jobs › ${name}`, aria: `Back to Jobs, ${name}` };
}
