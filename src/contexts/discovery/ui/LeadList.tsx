// The Leads stage's list, full width: one row per lead with a tick box, the score, the role (and
// the company and place under it), why it matches, which source found it and when. A row opens the
// lead in the drawer; the tick boxes feed "Prepare N CVs".

import type { ReactNode } from "react";

import { ago, Checkbox, DataTable, ScoreBadge, scoreLabel, Spinner, type DataColumn } from "@/shared";

import { jobLabel } from "../domain/queue";
import { metaLine, reasonsLine, sourceName } from "../domain/text";
import type { Job, Lead, Source } from "../domain/types";
import { selectLead, setChecked } from "../hooks/todayStore";

interface Props {
  shown: readonly Lead[];
  sources: readonly Source[];
  /** The lead the drawer shows. */
  openId: number | undefined;
  checked: readonly number[];
  jobs: Readonly<Record<number, Job>>;
  /** What the table says when no lead passes the filters. */
  empty?: ReactNode;
}

/** A lead's row as assistive technology names it (tests find rows by it). */
export function leadRowLabel(lead: Lead): string {
  return `${lead.company}: ${lead.role}, score ${scoreLabel(lead.score)}`;
}

export function LeadList({ shown, sources, openId, checked, jobs, empty }: Props) {
  const now = new Date();
  const sourceOf = (lead: Lead) => sources.find((s) => s.id === lead.sourceId);

  const columns: DataColumn<Lead>[] = [
    {
      key: "pick",
      header: <span className="sr-only">Select</span>,
      className: "w-8 pr-0",
      // A tick never opens the row.
      cell: (lead) => (
        <Checkbox
          id={`lead-check-${lead.id}`}
          label={null}
          aria-label={`Select ${lead.company}: ${lead.role}`}
          checked={checked.includes(lead.id)}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => setChecked(lead.id, e.target.checked)}
        />
      ),
    },
    { key: "score", header: "Score", className: "w-14", cell: (lead) => <ScoreBadge score={lead.score} /> },
    {
      key: "role",
      header: "Role",
      // Takes the rest of the width; its text is cut with an ellipsis, the whole of it in the title.
      className: "w-full max-w-0",
      cell: (lead) => {
        const where = `${lead.company} · ${metaLine(lead.location, null)}`;
        const job = jobs[lead.id];
        return (
          <span id={`lead-${lead.id}`} className="flex min-w-0 flex-col gap-0.5">
            <span title={lead.role} className="truncate font-semibold text-ink">
              {lead.role}
            </span>
            <span title={where} className="truncate text-small text-muted">
              {where}
            </span>
            {job && (
              <span className="flex items-center gap-1 text-small text-accent">
                <Spinner />
                {jobLabel(job)}
              </span>
            )}
          </span>
        );
      },
    },
    {
      key: "why",
      header: "Why",
      className: "hidden max-w-[320px] min-[1100px]:table-cell",
      cell: (lead) => {
        const reasons = reasonsLine(lead.reasons);
        return (
          <span title={reasons} className="block truncate text-small text-muted">
            {reasons}
          </span>
        );
      },
    },
    {
      key: "source",
      header: "Source",
      className: "text-small text-muted",
      cell: (lead) => {
        const source = sourceOf(lead);
        return source ? sourceName(source) : "";
      },
    },
    {
      key: "found",
      header: "Found",
      className: "text-small text-subtle",
      cell: (lead) => {
        const found = new Date(lead.foundAt);
        return Number.isNaN(found.getTime()) ? "" : ago(now, found);
      },
    },
  ];

  return (
    <DataTable
      id="queue-list"
      label="Leads"
      columns={columns}
      rows={shown}
      rowKey={(lead) => String(lead.id)}
      rowLabel={leadRowLabel}
      onOpen={(lead) => selectLead(lead.id)}
      openKey={openId === undefined ? null : String(openId)}
      empty={empty}
    />
  );
}
