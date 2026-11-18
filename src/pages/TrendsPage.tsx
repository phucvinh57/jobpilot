import { Plus } from "lucide-react";
import { useState } from "react";

import { Button, Caption, Page, Tabs } from "@/components/ui/kit";
import { LoadError } from "@/components/ProblemHelp";

import { useTrends } from "@/hooks/trends/useTrends";
import { Challenges } from "@/components/trends/Challenges";
import { NewResearchDialog } from "@/components/trends/NewResearchDialog";
import { ReportView } from "@/components/trends/ReportView";
import { Reports } from "@/components/trends/Reports";
import { SourcesSection } from "@/components/trends/SourcesSection";

type Tab = "reports" | "sources";

const TABS: readonly { value: Tab; label: string }[] = [
  { value: "reports", label: "Reports" },
  { value: "sources", label: "Sources" },
];

/** Market trends: where the market of your place is going (local, then global), your gap against
 *  your background, then your wishes against the market, from the sources you add. The list has
 *  two tabs, the saved reports and the sources; "New research" (the one primary button) asks for
 *  the period in a dialog, collects its items, then asks the AI provider for a digest. A click on a
 *  report opens it: every point links to the stored items it rests on. */
export function TrendsPage() {
  const t = useTrends();
  const { data, report } = t;
  const [tab, setTab] = useState<Tab>("reports");
  const [creating, setCreating] = useState(false);

  const loadError = <LoadError error={t.loadError} title="The saved data could not be read" onRetry={t.reload} />;

  if (report) {
    return (
      <div id="trends" className="h-full">
        <ReportView
          loading={report.loading}
          digest={report.digest}
          chart={report.chart}
          row={report.row}
          adds={t.adds}
          step={t.running?.step}
          busy={t.busy}
          exporting={t.exporting}
          onBack={t.back}
          onExport={t.exportPdf}
          onRegenerate={(period) => void t.regenerateFor(period)}
          onRerun={(period) => void t.rerun(period)}
        >
          {loadError}
        </ReportView>
      </div>
    );
  }

  return (
    <div id="trends" className="h-full">
      <Page
        label="Trends"
        title="Trends"
        subtitle={data && <span id="trends-summary">{data.summary}</span>}
        actions={
          <Button
            id="trends-new"
            variant="primary"
            icon={Plus}
            aria-haspopup="dialog"
            busy={t.busy}
            onClick={() => setCreating(true)}
          >
            New research
          </Button>
        }
      >
        {loadError}
        <div id="trends-tabs">
          <Tabs<Tab> label="Trends" tabs={TABS} value={tab} onChange={setTab} />
        </div>
        {data ? (
          tab === "reports" ? (
            <>
              <Challenges
                challenges={data.challenges}
                busy={t.busy}
                onContinue={(period) => void t.resume(period)}
                onWithout={(period) => void t.regenerateFor(period)}
              />
              <Reports
                reports={data.history}
                cadence={t.cadence}
                onCadence={t.chooseCadence}
                running={t.running}
                exporting={t.exporting}
                deleting={t.deleting}
                onOpen={t.pickReport}
                onRerun={(period) => void t.rerun(period)}
                onExport={t.exportPdf}
                onDelete={t.remove}
              />
            </>
          ) : (
            <SourcesSection sources={data.sources} kinds={data.sourceKinds} />
          )
        ) : (
          t.loading && <Caption>Loading…</Caption>
        )}
      </Page>
      <NewResearchDialog
        open={creating}
        cadence={t.cadence}
        onCadence={t.chooseCadence}
        periodLabel={t.periodLabel}
        onStart={() => {
          if (t.collect()) {
            setCreating(false);
          }
        }}
        onClose={() => setCreating(false)}
      />
    </div>
  );
}
