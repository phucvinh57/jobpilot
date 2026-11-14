import { useState } from "react";

import { useNav, type Origin } from "@/shared";

import { DEFAULT_TAB, primaryClick, type Tab } from "../domain/tabs";
import { useApplicationHeader, type HeaderModel } from "./useApplicationHeader";
import { useApplySession, type ApplySession } from "./useApplySession";

export interface ApplicationScreenModel {
  head: HeaderModel;
  apply: ApplySession;
  tab: Tab;
  setTab: (tab: Tab) => void;
  /** Where the breadcrumb leads back to. */
  from: Origin;
  /** The click on the one primary button. */
  onPrimary: () => void;
  /** "Next job": back to Today, on its best lead. */
  nextJob: () => void;
}

/** Everything the Application screen shows and does for the open application `id`. */
export function useApplicationScreen(id: number): ApplicationScreenModel {
  const from = useNav((s) => s.detailFrom);
  const openLead = useNav((s) => s.openLead);
  const [tab, setTab] = useState<Tab>(DEFAULT_TAB);
  const head = useApplicationHeader(id);
  const apply = useApplySession(id, head.setPrimary);

  const onPrimary = () => {
    const primary = head.primary;
    if (primary === null) {
      return;
    }
    const click = primaryClick(primary.action, tab);
    setTab(click.tab);
    if (!click.run) {
      return;
    }
    switch (primary.action) {
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
  };

  return { head, apply, tab, setTab, from, onPrimary, nextJob: () => openLead(undefined) };
}
