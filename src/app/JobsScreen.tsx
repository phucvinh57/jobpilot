// The Jobs page: every job from found to finished. The header and the Leads stage come from the
// discovery context, the CVs / Applied / In progress / Done stages from the applications context;
// a drawer at the right shows the open lead or application. At most one drawer is open, and it holds
// the one primary button while it is.

import clsx from "clsx";

import { appStages, ApplicationDrawer, ApplicationsStage, useBoard } from "@/contexts/applications";
import {
  LeadDrawer,
  LeadsHeaderLine,
  LeadsStage,
  LinkBox,
  NeedsYou,
  SearchNowButton,
  SourcesLine,
  useLeadsLifecycle,
  useLeadsSummary,
} from "@/contexts/discovery";
import { Page, StageBar, useNav, type Stage } from "@/shared";

import { drawerPad, stageItems } from "./jobs";

export function JobsScreen() {
  useLeadsLifecycle();
  const summary = useLeadsSummary();
  const board = useBoard();
  const stage = useNav((s) => s.stage);
  const drawer = useNav((s) => s.drawer);
  const showStage = useNav((s) => s.showStage);
  const appOpen = drawer?.kind === "app";

  return (
    <div className={clsx("h-full", drawerPad(drawer))}>
      <Page label="Jobs" title="Jobs" subtitle={<LeadsHeaderLine />} actions={<SearchNowButton otherDrawerOpen={appOpen} />}>
        <LinkBox />
        <SourcesLine />
        <NeedsYou />
        <StageBar<Stage> label="Stages" stages={stageItems(summary.newLeads, appStages(board.data))} value={stage} onChange={showStage} />
        {stage === "leads" ? <LeadsStage otherDrawerOpen={appOpen} /> : <ApplicationsStage stage={stage} />}
      </Page>
      <LeadDrawer />
      <ApplicationDrawer />
    </div>
  );
}
