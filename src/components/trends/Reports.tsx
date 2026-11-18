import { useState } from "react";
import { ChartPie, FileDown, RefreshCw, Trash2 } from "lucide-react";

import {
  Button,
  Caption,
  type DataColumn,
  DataTable,
  Dialog,
  EmptyState,
  Section,
  Segmented,
} from "@/components/ui/kit";

import { CADENCES, noReportsCaption, reportName } from "@/lib/trends/format";
import type { Cadence, ReportRow } from "@/types/trends";
import { RunStatus } from "./RunStatus";

/** The saved reports of one cadence, newest period first: a click on a row opens the report;
 *  each can be collected again (it replaces the report), saved as a PDF or deleted. The row of the
 *  period a run is building says the step it is at. */
export function Reports({
  reports,
  cadence,
  onCadence,
  running,
  exporting,
  deleting,
  onOpen,
  onRerun,
  onExport,
  onDelete,
}: {
  /** Every saved report of every cadence. */
  reports: readonly ReportRow[];
  /** The cadence whose reports are listed (also the period of a new research). */
  cadence: Cadence;
  onCadence: (cadence: Cadence) => void;
  /** The trends run in flight (no rerun meanwhile): the period it builds and its step. */
  running: { period: string; step: string } | undefined;
  exporting: boolean;
  deleting: boolean;
  onOpen: (id: number) => void;
  onRerun: (period: string) => void;
  onExport: (id: number, period: string) => void;
  /** Resolves `true` once the report is gone. */
  onDelete: (id: number) => Promise<boolean>;
}) {
  const [confirming, setConfirming] = useState<ReportRow>();
  const rows = reports.filter((r) => r.cadence === cadence);
  const busy = running !== undefined;
  // The run's step is said in the row of its period; above the list while no listed row is its.
  const building = (r: ReportRow) => running !== undefined && r.period === running.period;
  const orphan = running !== undefined && !rows.some(building) ? running : undefined;

  const columns: DataColumn<ReportRow>[] = [
    {
      key: "period",
      header: "Period",
      cell: (r) => (
        <span className="flex min-w-0 flex-col">
          <span className="selectable truncate font-semibold text-ink">{r.periodTitle}</span>
          {r.periodTitle !== r.period && <span className="text-small text-muted">{r.period}</span>}
        </span>
      ),
    },
    { key: "points", header: "Points", className: "w-20 tabular-nums", cell: (r) => r.points },
    {
      key: "when",
      header: "Updated",
      className: "text-muted",
      cell: (r) => (running && building(r) ? <RunStatus step={running.step} /> : r.when),
    },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "text-right",
      cell: (r) => {
        const name = reportName(r);
        return (
          <span className="flex justify-end gap-2 whitespace-nowrap">
            <Button
              id={`trends-report-${r.id}-rerun`}
              size="compact"
              icon={RefreshCw}
              aria-label={`Rerun ${name}`}
              title="Collect this period again and rebuild its report (it replaces this one)"
              disabled={busy || !r.rerun}
              onClick={() => onRerun(r.period)}
            >
              Rerun
            </Button>
            <Button
              id={`trends-report-${r.id}-pdf`}
              size="compact"
              icon={FileDown}
              aria-label={`Export PDF of ${name}`}
              disabled={exporting}
              onClick={() => onExport(r.id, r.period)}
            >
              Export PDF
            </Button>
            <Button
              id={`trends-report-${r.id}-delete`}
              variant="danger"
              size="compact"
              icon={Trash2}
              aria-label={`Delete ${name}`}
              aria-haspopup="dialog"
              disabled={deleting}
              onClick={() => setConfirming(r)}
            >
              Delete
            </Button>
          </span>
        );
      },
    },
  ];

  return (
    <Section
      id="trends-reports"
      title="Reports"
      hint="One report per period: click one to read it. Rerun collects the period again and replaces its report."
    >
      <div id="trends-reports-cadence">
        <Segmented label="Reports period" choices={CADENCES} value={cadence} onChange={onCadence} />
      </div>
      {orphan && <RunStatus step={orphan.step} period={orphan.period} />}
      {rows.length === 0 ? (
        <EmptyState icon={ChartPie} title="No reports yet" body={noReportsCaption(cadence)} />
      ) : (
        <DataTable
          id="trends-reports-table"
          label="Reports"
          columns={columns}
          rows={rows}
          rowKey={(r) => String(r.id)}
          rowLabel={reportName}
          onOpen={(r) => onOpen(r.id)}
        />
      )}
      <Dialog
        id="trends-report-delete"
        open={confirming !== undefined}
        width="sm"
        title="Delete this report?"
        description={confirming && reportName(confirming)}
        busy={deleting}
        onClose={() => setConfirming(undefined)}
        footer={
          <>
            <Button id="trends-report-delete-cancel" disabled={deleting} onClick={() => setConfirming(undefined)}>
              Cancel
            </Button>
            <Button
              id="trends-report-delete-confirm"
              variant="primary"
              busy={deleting}
              onClick={() => {
                if (confirming) {
                  void onDelete(confirming.id).then((gone) => gone && setConfirming(undefined));
                }
              }}
            >
              Delete
            </Button>
          </>
        }
      >
        <Caption>The report is removed. Collecting its period again builds a new one.</Caption>
      </Dialog>
    </Section>
  );
}
