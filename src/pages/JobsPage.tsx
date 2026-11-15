// The Jobs page: every job from found to finished. The header and the Leads stage come from the
// components in `components/discovery`, the CVs / Applied / In progress / Done stages from `components/applications`;
// a drawer at the right shows the open lead or application. At most one drawer is open, and it holds
// the one primary button while it is.

import clsx from "clsx";

import { appStages } from "@/lib/applications/pipeline";
import { ApplicationDrawer } from "@/components/applications/ApplicationDrawer";
import { ApplicationsStage } from "@/components/applications/ApplicationsStage";
import { useBoard } from "@/hooks/applications/useBoard";
import { LeadDrawer } from "@/components/discovery/LeadDrawer";
import { LeadsHeaderLine, SearchNowButton, SourcesLine } from "@/components/discovery/JobsHeader";
import { LeadsStage } from "@/components/discovery/LeadsStage";
import { LinkBox } from "@/components/discovery/LinkBox";
import { NeedsYou } from "@/components/discovery/NeedsYou";
import { useLeadsLifecycle, useLeadsSummary } from "@/hooks/discovery/useToday";
import { Page, StageBar } from "@/components/ui/kit";
import { useNav, type Stage } from "@/stores/nav";

import { drawerPad, stageItems } from "@/lib/jobs";

export function JobsPage() {
  useLeadsLifecycle();
  const summary = useLeadsSummary();
  const board = useBoard();
  const stage = useNav((s) => s.stage);
  const drawer = useNav((s) => s.drawer);
  const showStage = useNav((s) => s.showStage);
  const appOpen = drawer?.kind === "app";

  return (
    <div className={clsx("h-full", drawerPad(drawer))}>
      <Page
        label="Jobs"
        title="Jobs"
        subtitle={<LeadsHeaderLine />}
        actions={<SearchNowButton otherDrawerOpen={appOpen} />}
      >
        <LinkBox />
        <SourcesLine />
        <NeedsYou />
        <StageBar<Stage>
          label="Stages"
          stages={stageItems(summary.newLeads, appStages(board.data))}
          value={stage}
          onChange={showStage}
        />
        {stage === "leads" ? (
          <LeadsStage otherDrawerOpen={appOpen} />
        ) : (
          <ApplicationsStage stage={stage} board={board} />
        )}
      </Page>
      <LeadDrawer />
      <ApplicationDrawer />
    </div>
  );
}
