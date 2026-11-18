import { FileDown, PenLine, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";

import { Button, Caption, Heading, LinkButton, Page } from "@/components/ui/kit";

import { digestHint, reportName } from "@/lib/trends/format";
import type { Chart, Digest, ReportRow } from "@/types/trends";
import type { PreferenceAdds } from "@/hooks/trends/usePreferenceAdds";
import { DigestCard } from "./DigestCard";
import { RunStatus } from "./RunStatus";
import { SkillChart } from "./SkillChart";

/** One saved report: its digest with the skill chart beside it. Opened from a row of the list;
 *  the breadcrumb goes back. Its actions are all secondary: save it as a PDF, write the digest
 *  again from the stored items, or collect its period again. The last two start a run, so they
 *  wait while any run is in flight; the step of a run is said here only when it builds this
 *  report's period. */
export function ReportView({
  loading,
  digest,
  chart,
  row,
  adds,
  running,
  exporting,
  onBack,
  onExport,
  onRegenerate,
  onRerun,
  children,
}: {
  /** The report has not been read yet. */
  loading: boolean;
  digest: Digest | null;
  chart: Chart | null;
  /** Its row in the list (its period); `undefined` when it is not there. */
  row: ReportRow | undefined;
  adds: PreferenceAdds;
  /** The trends run in flight (no second one meanwhile): the period it builds and its step. */
  running: { period: string; step: string } | undefined;
  exporting: boolean;
  onBack: () => void;
  onExport: (id: number, period: string) => void;
  onRegenerate: (period: string) => void;
  onRerun: (period: string) => void;
  /** Above the report (a failed read, a collection that waits for the user). */
  children?: ReactNode;
}) {
  const title = digest?.title ?? row?.title ?? "Report";
  const shown = !loading && digest !== null && row !== undefined ? row : undefined;
  const busy = running !== undefined;
  const step = running !== undefined && row?.period === running.period ? running.step : undefined;
  return (
    <Page
      label="Trends report"
      title={
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex min-w-0 items-center gap-1.5 text-small text-muted">
            <LinkButton id="trends-back" aria-label="Back to Trends" onClick={onBack} className="shrink-0">
              Trends
            </LinkButton>
            <span aria-hidden>/</span>
            <span className="min-w-0 truncate">{row ? reportName(row) : title}</span>
          </div>
          <Heading level="h1">{title}</Heading>
        </div>
      }
      subtitle={
        (digest || step) && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {digest && <span>{digestHint(digest.when, digest.itemsRead)}</span>}
            {step && <RunStatus step={step} />}
          </div>
        )
      }
      actions={
        <>
          <Button
            id="trends-export-pdf"
            size="compact"
            icon={FileDown}
            busy={exporting}
            disabled={!shown}
            onClick={() => shown && onExport(shown.id, shown.period)}
          >
            Export PDF
          </Button>
          <Button
            id="trends-regenerate"
            size="compact"
            icon={PenLine}
            title="Write the digest again from the stored items, without collecting"
            disabled={!shown || busy}
            onClick={() => shown && onRegenerate(shown.period)}
          >
            Rewrite digest
          </Button>
          <Button
            id="trends-rerun"
            size="compact"
            icon={RefreshCw}
            title="Collect this period again and rebuild its report (it replaces this one)"
            disabled={!shown?.rerun || busy}
            onClick={() => shown && onRerun(shown.period)}
          >
            Rerun
          </Button>
        </>
      }
    >
      {children}
      {loading ? (
        <Caption>Loading…</Caption>
      ) : (
        // The digest on the left, the skill chart beside it (below it when narrow).
        <div className="flex flex-wrap items-start gap-5">
          <div className="flex min-w-[360px] flex-[1.2_1_420px] flex-col">
            <DigestCard digest={digest} adds={adds} />
          </div>
          {chart && (
            <div className="flex min-w-[340px] flex-[1_1_380px] flex-col gap-5">
              <SkillChart key={digest?.reportId ?? 0} chart={chart} adds={adds} />
            </div>
          )}
        </div>
      )}
    </Page>
  );
}
