import { useState } from "react";
import { Eye, FileDown, RefreshCw, Trash2 } from "lucide-react";

import { Button, Caption, type DataColumn, DataTable, Dialog, Section, Segmented } from "@/components/ui/kit";

import { CADENCES, noReportsCaption, reportName } from "@/lib/trends/format";
import type { Cadence, ReportRow } from "@/types/trends";

/** The saved reports of one cadence, newest period first: open one in the digest card, collect its
 *  period again (it replaces the report), save it as a PDF or delete it. The filter follows the
 *  cadence chosen above, and can be switched on its own. */
export function Reports({
  reports,
  cadence,
  shown,
  busy,
  exporting,
  deleting,
  onOpen,
  onRerun,
  onExport,
  onDelete,
}: {
  /** Every saved report of every cadence. */
  reports: readonly ReportRow[];
  /** The cadence chosen in the controls: the filter starts there. */
  cadence: Cadence;
  /** The report in the digest card. */
  shown: number | undefined;
  /** A trends run is in flight: no rerun. */
  busy: boolean;
  exporting: boolean;
  deleting: boolean;
  onOpen: (id: number) => void;
  onRerun: (period: string) => void;
  onExport: (id: number, period: string) => void;
  /** Resolves `true` once the report is gone. */
  onDelete: (id: number) => Promise<boolean>;
}) {
  const [filter, setFilter] = useState(cadence);
  // A new cadence above moves the filter with it.
  const [followed, setFollowed] = useState(cadence);
  if (followed !== cadence) {
    setFollowed(cadence);
    setFilter(cadence);
  }
  const [confirming, setConfirming] = useState<ReportRow>();
  const rows = reports.filter((r) => r.cadence === filter);

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
    { key: "when", header: "Updated", className: "text-muted", cell: (r) => r.when },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "text-right",
      cell: (r) => {
        const name = reportName(r);
        return (
          <span className="flex flex-wrap justify-end gap-2">
            <Button
              id={`trends-report-${r.id}-open`}
              size="compact"
              icon={Eye}
              aria-label={`Open ${name}`}
              onClick={() => onOpen(r.id)}
            >
              Open
            </Button>
            <Button
              id={`trends-report-${r.id}-rerun`}
              size="compact"
              icon={RefreshCw}
              aria-label={`Rerun ${name}`}
              title="Collect this period again and rebuild its report (it replaces this one)"
              disabled={busy}
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
      hint="One report per period. Rerun collects the period again and replaces its report."
    >
      <div id="trends-reports-cadence">
        <Segmented label="Reports period" choices={CADENCES} value={filter} onChange={setFilter} />
      </div>
      {rows.length === 0 ? (
        <Caption>{noReportsCaption(filter)}</Caption>
      ) : (
        <DataTable
          label="Reports"
          columns={columns}
          rows={rows}
          rowKey={(r) => String(r.id)}
          rowLabel={reportName}
          onOpen={(r) => onOpen(r.id)}
          openKey={shown === undefined ? null : String(shown)}
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
