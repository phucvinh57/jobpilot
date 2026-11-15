import type { PrimaryKey } from "../domain/types";
import type { HeaderModel } from "./useApplicationHeader";

/** Run the primary action `action` (the screen or drawer already decided it runs now). */
export function runPrimary(
  action: PrimaryKey,
  head: Pick<HeaderModel, "generate" | "approve">,
  apply: { prepare: () => void; confirm: () => void },
): void {
  switch (action) {
    case "generate":
      head.generate();
      break;
    case "approveCv":
      head.approve();
      break;
    case "reviewAndSubmit":
      apply.prepare();
      break;
    case "continue":
    case "submit":
      apply.confirm();
      break;
  }
}
