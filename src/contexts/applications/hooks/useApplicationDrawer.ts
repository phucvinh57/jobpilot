import { useAction, useNav, type Action } from "@/shared";

import { mark, setRound } from "../api/applicationsApi";
import { primaryClick } from "../domain/tabs";
import type { ApplicationHeader, MarkKey } from "../domain/types";
import { requestFullView } from "./fullView";
import { runPrimary } from "./primary";
import { useApplicationHeader, type HeaderModel } from "./useApplicationHeader";

export interface ApplicationDrawerModel {
  head: HeaderModel;
  /** The header of `id` (`undefined` while it loads, also while another application's is shown). */
  header: ApplicationHeader | null | undefined;
  /** The click on the one primary button. Generate and Approve CV run here; the apply steps need
   *  the review, so they open the full view on its Apply tab ("Review & Submit" starts there). */
  onPrimary: () => void;
  /** "Open full view". */
  expand: () => void;
  /** "Next job": the best Lead. */
  nextJob: () => void;
  close: () => void;
  setRound: Action<[round: string | null], ApplicationHeader>;
  mark: Action<[mark: MarkKey], ApplicationHeader>;
}

/** The drawer of application `id` on the Jobs page. */
export function useApplicationDrawer(id: number): ApplicationDrawerModel {
  const openApplication = useNav((s) => s.openApplication);
  const openLead = useNav((s) => s.openLead);
  const close = useNav((s) => s.closeDrawer);
  const head = useApplicationHeader(id);
  const roundAction = useAction((round: string | null) => setRound(id, round));
  const markAction = useAction((key: MarkKey) => mark(id, key));
  const header = head.header === null ? null : head.header?.id === id ? head.header : undefined;

  const onPrimary = () => {
    const primary = head.primary;
    if (primary === null) {
      return;
    }
    // The drawer shows the timeline: as on the full view's Timeline tab.
    const click = primaryClick(primary.action, "timeline");
    if (click.tab === "apply") {
      requestFullView({ id, tab: "apply", prepare: click.run });
      openApplication(id);
      return;
    }
    runPrimary(primary.action, head, { prepare: () => undefined, confirm: () => undefined });
  };

  return {
    head,
    header,
    onPrimary,
    expand: () => openApplication(id),
    nextJob: () => openLead("best"),
    close,
    setRound: roundAction,
    mark: markAction,
  };
}
