// The tabs of the Application screen and what a click on the primary button does with them.

import type { PrimaryKey } from "./types";

export type Tab = "jd" | "cv" | "apply" | "timeline";

export const TABS: readonly { value: Tab; label: string }[] = [
  { value: "jd", label: "JD" },
  { value: "cv", label: "CV" },
  { value: "apply", label: "Apply" },
  { value: "timeline", label: "Timeline" },
];

/** An application opens on its CV: that is what the user reviews first. */
export const DEFAULT_TAB: Tab = "cv";

export interface PrimaryClick {
  /** The tab to show. */
  tab: Tab;
  /** Run the action now (`false`: only bring the tab into view). */
  run: boolean;
}

/**
 * What the primary button does when pressed on tab `current`. Generate and Approve CV act where the
 * user is. Review & Submit opens the Apply tab and starts. Submitting is an explicit click on what the
 * user is looking at, so Continue / Submit from another tab only bring the open review into view.
 */
export function primaryClick(action: PrimaryKey, current: Tab): PrimaryClick {
  switch (action) {
    case "generate":
    case "approveCv":
      return { tab: current, run: true };
    case "reviewAndSubmit":
      return { tab: "apply", run: true };
    case "continue":
    case "submit":
      return { tab: "apply", run: current === "apply" };
  }
}
