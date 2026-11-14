// The breadcrumb of the Application screen leads back to the screen it was opened from.

import type { Origin, Screen } from "@/shared";

export interface Crumb {
  id: string;
  label: string;
  aria: string;
  target: Screen;
}

/** The breadcrumb for an application opened from `from` (Today, else the Pipeline). */
export function crumbFor(from: Origin): Crumb {
  return from === "today"
    ? { id: "breadcrumb-today", label: "Today", aria: "Back to Today", target: "today" }
    : { id: "breadcrumb-pipeline", label: "Pipeline", aria: "Back to the Pipeline", target: "pipeline" };
}
