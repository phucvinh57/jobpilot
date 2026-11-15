import clsx from "clsx";

import { Caption, Dialog } from "@/components/ui/kit";

import { reportRowName } from "@/lib/trends/format";
import type { ReportRow } from "@/types/trends";

/** Past digests, newest first, in a dialog; picking one shows it again (and closes the dialog). */
export function History({
  open,
  reports,
  shown,
  onSelect,
  onClose,
}: {
  open: boolean;
  reports: readonly ReportRow[];
  /** The report on screen. */
  shown: number | undefined;
  onSelect: (id: number) => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      id="trends-history"
      open={open}
      width="lg"
      title="Earlier digests"
      description="Past digests, newest first. Click one to read it again."
      onClose={onClose}
    >
      {reports.length === 0 && <Caption>No digests yet.</Caption>}
      <ul aria-label="Earlier digests" className="flex flex-col">
        {reports.map((report) => {
          const selected = report.id === shown;
          return (
            <li key={report.id}>
              <button
                id={`trends-report-${report.id}`}
                type="button"
                aria-label={reportRowName(report.title, report.when)}
                aria-current={selected ? "true" : undefined}
                onClick={() => onSelect(report.id)}
                className={clsx(
                  "flex w-full cursor-pointer items-center justify-between gap-2 px-3 py-2 text-left text-body transition-colors",
                  selected ? "bg-accent-soft font-semibold text-accent-dark" : "text-ink hover:bg-track",
                )}
              >
                <span className="min-w-0 truncate">{report.title}</span>
                <span className="shrink-0 text-small text-muted">{report.when}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </Dialog>
  );
}
