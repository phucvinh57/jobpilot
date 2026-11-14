import { useEffect } from "react";

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
}

/** What the user records by hand in the drawer of one application. Used inside content keyed by the
 *  application, so a failure or a busy state never carries over to another one. */
export interface ApplicationMarks {
  setRound: Action<[round: string | null], ApplicationHeader>;
  mark: Action<[mark: MarkKey], ApplicationHeader>;
}

export function useApplicationMarks(id: number): ApplicationMarks {
  return {
    setRound: useAction((round: string | null) => setRound(id, round)),
    mark: useAction((key: MarkKey) => mark(id, key)),
  };
}

/** The drawer of application `id` on the Jobs page. While it shows `id`, the page shows the stage
 *  the application is in: whoever opened it need not know the stage, and a mark that moves it
 *  (Applied -> In progress) moves the page with it. */
export function useApplicationDrawer(id: number): ApplicationDrawerModel {
  const openApplication = useNav((s) => s.openApplication);
  const openLead = useNav((s) => s.openLead);
  const close = useNav((s) => s.closeDrawer);
  const shown = useNav((s) => s.drawer?.kind === "app" && s.drawer.id === id);
  const head = useApplicationHeader(id);
  const header = head.header === null ? null : head.header?.id === id ? head.header : undefined;

  const stage = header?.stage;
  useEffect(() => {
    if (shown && stage !== undefined && stage !== useNav.getState().stage) {
      // The stage only: the drawer stays open (`showStage` would close it).
      useNav.setState({ stage });
    }
  }, [shown, stage]);

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
  };
}
