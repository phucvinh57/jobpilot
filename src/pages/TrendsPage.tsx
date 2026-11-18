import { RefreshCw } from "lucide-react";

import { Button, Caption, Page } from "@/components/ui/kit";

import { useTrends } from "@/hooks/trends/useTrends";
import { Challenges } from "@/components/trends/Challenges";
import { Controls } from "@/components/trends/Controls";
import { DigestCard } from "@/components/trends/DigestCard";
import { NoticeView } from "@/components/trends/NoticeView";
import { Reports } from "@/components/trends/Reports";
import { SkillChart } from "@/components/trends/SkillChart";
import { SourcesSection } from "@/components/trends/SourcesSection";

/** Market trends: where the market of your place is going (local, then global), your gap against
 *  your background, then your wishes against the market, from the sources you add. "Collect & build digest"
 *  (the one primary button) collects the period's items, then asks the AI provider for a digest;
 *  every point links to the stored items it rests on. */
export function TrendsPage() {
  const t = useTrends();
  const { data } = t;
  return (
    <div id="trends" className="h-full">
      <Page
        label="Trends"
        title="Trends"
        subtitle={data && <span id="trends-summary">{data.summary}</span>}
        actions={
          <Button
            id="trends-regenerate"
            size="compact"
            icon={RefreshCw}
            aria-label="Regenerate from stored items"
            title="Write the digest again from the stored items, without collecting"
            disabled={t.busy || !t.ready}
            onClick={() => void t.regenerate()}
          >
            Refresh
          </Button>
        }
      >
        <Controls
          cadence={t.cadence}
          onCadence={t.chooseCadence}
          periodLabel={data?.period.label}
          busy={t.busy}
          ready={t.ready}
          step={t.step}
          onCollect={() => void t.collect()}
        />
        {t.notice && <NoticeView notice={t.notice} />}
        {data && (
          <Challenges
            challenges={data.challenges}
            busy={t.busy}
            onContinue={(period) => void t.resume(period)}
            onWithout={(period) => void t.regenerateFor(period)}
          />
        )}
        {data ? (
          <>
            {/* The digest on the left, the skill chart beside it (below it when narrow). */}
            <div className="flex flex-wrap items-start gap-5">
              <div className="flex min-w-[360px] flex-[1.2_1_420px] flex-col">
                <DigestCard digest={data.digest} adds={t.adds} exporting={t.exporting} onExportPdf={t.exportShown} />
              </div>
              {data.chart && (
                <div className="flex min-w-[340px] flex-[1_1_380px] flex-col gap-5">
                  <SkillChart key={data.digest?.reportId ?? 0} chart={data.chart} adds={t.adds} />
                </div>
              )}
            </div>
            <Reports
              reports={data.history}
              cadence={t.cadence}
              shown={t.shownReport}
              busy={t.busy}
              exporting={t.exporting}
              deleting={t.deleting}
              onOpen={t.pickReport}
              onRerun={(period) => void t.rerun(period)}
              onExport={t.exportPdf}
              onDelete={t.remove}
            />
            <SourcesSection sources={data.sources} kinds={data.sourceKinds} />
          </>
        ) : (
          t.loading && <Caption>Loading…</Caption>
        )}
      </Page>
    </div>
  );
}
