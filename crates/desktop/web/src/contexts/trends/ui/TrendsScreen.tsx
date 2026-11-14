import { RefreshCw } from "lucide-react";

import { Button, Caption, FilterChip, Page } from "@/shared";

import { useTrends } from "../hooks";
import { Challenges } from "./Challenges";
import { Controls } from "./Controls";
import { DigestCard } from "./DigestCard";
import { History } from "./History";
import { NoticeView } from "./NoticeView";
import { SearchSection } from "./SearchSection";
import { SkillChart } from "./SkillChart";
import { SourcesSection } from "./SourcesSection";

/** Market trends: where the market is going, from the sources you add. "Collect & build digest"
 *  (the one primary button) collects the period's items, then asks the AI provider for a digest;
 *  every point links to the stored items it rests on. */
export function TrendsScreen() {
  const t = useTrends();
  const { data } = t;
  return (
    <div id="trends" className="h-full">
      <Page
        label="Trends"
        title="Trends"
        subtitle={data && <span id="trends-summary">{data.summary}</span>}
        actions={
          <>
            <FilterChip id="trends-history-toggle" selected={t.showHistory} onClick={t.toggleHistory}>
              Earlier digests
            </FilterChip>
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
          </>
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
        {t.notice && <NoticeView notice={t.notice} onRetry={(job) => void t.retry(job)} />}
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
              <div className="flex min-w-[380px] flex-[1.4_1_440px] flex-col">
                <DigestCard digest={data.digest} adds={t.adds} />
              </div>
              {data.chart && (
                <div className="flex min-w-[320px] flex-[1_1_340px] flex-col gap-5">
                  <SkillChart chart={data.chart} adds={t.adds} />
                </div>
              )}
            </div>
            <SourcesSection sources={data.sources} kinds={data.sourceKinds} />
            {t.showHistory && <History reports={data.history} shown={t.shownReport} onSelect={t.selectReport} />}
            <SearchSection keys={data.searchKeys} />
          </>
        ) : (
          t.loading && <Caption>Loading…</Caption>
        )}
      </Page>
    </div>
  );
}
