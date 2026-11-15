import { useEffect, useState } from "react";

import { useNav } from "@/stores/nav";

import { DEFAULT_TAB, primaryClick, type Tab } from "@/lib/applications/tabs";
import { peekFullView, takeFullView } from "./fullView";
import { runPrimary } from "./primary";
import { useApplicationHeader, type HeaderModel } from "./useApplicationHeader";
import { useApplySession, type ApplySession } from "./useApplySession";

export interface ApplicationScreenModel {
  head: HeaderModel;
  apply: ApplySession;
  tab: Tab;
  setTab: (tab: Tab) => void;
  /** The click on the one primary button. */
  onPrimary: () => void;
  /** "Next job": the best Lead, in the drawer of the Jobs page. */
  nextJob: () => void;
}

/** Everything the Application screen shows and does for the open application `id`. */
export function useApplicationScreen(id: number): ApplicationScreenModel {
  const openLead = useNav((s) => s.openLead);
  // Opened from the drawer's "Review & Submit": start on the Apply tab and run it there.
  const [start] = useState(() => peekFullView(id));
  const [tab, setTab] = useState<Tab>(start?.tab ?? DEFAULT_TAB);
  const head = useApplicationHeader(id);
  const apply = useApplySession(id, head.setPrimary);

  const prepare = apply.prepare;
  useEffect(() => {
    if (start !== undefined && takeFullView(id) && start.prepare) {
      prepare();
    }
  }, [id, start, prepare]);

  const onPrimary = () => {
    const primary = head.primary;
    if (primary === null) {
      return;
    }
    const click = primaryClick(primary.action, tab);
    setTab(click.tab);
    if (click.run) {
      runPrimary(primary.action, head, apply);
    }
  };

  return { head, apply, tab, setTab, onPrimary, nextJob: () => openLead("best") };
}
