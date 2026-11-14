// The left pane: "New leads" and the cards, which scroll on their own. A card has a tick box (shown
// on hover, or once any box is ticked), the score badge, the role, where it is, why it matches, and
// which source found it and when.

import clsx from "clsx";

import { ago, Checkbox, Heading, ScoreBadge, scoreLabel, Spinner } from "@/shared";

import { jobLabel } from "../domain/queue";
import { metaLine, reasonsLine, sourceName } from "../domain/text";
import type { Job, Lead, Source } from "../domain/types";
import { selectLead, setChecked } from "../hooks/todayStore";

function LeadCard({
  lead,
  source,
  selected,
  checked,
  anyChecked,
  job,
  now,
}: {
  lead: Lead;
  source: Source | undefined;
  selected: boolean;
  checked: boolean;
  anyChecked: boolean;
  job: Job | undefined;
  now: Date;
}) {
  const reasons = reasonsLine(lead.reasons);
  const found = new Date(lead.foundAt);
  return (
    <li
      id={`lead-card-${lead.id}`}
      className={clsx(
        "group flex items-center rounded-banner border transition-colors",
        selected ? "border-accent bg-surface" : "border-border bg-transparent hover:bg-surface",
      )}
    >
      <div className={clsx("pl-3 transition-opacity", !anyChecked && "opacity-0 group-focus-within:opacity-100 group-hover:opacity-100")}>
        <Checkbox
          id={`lead-check-${lead.id}`}
          label={null}
          aria-label={`Select ${lead.company}: ${lead.role}`}
          checked={checked}
          onChange={(e) => setChecked(lead.id, e.target.checked)}
        />
      </div>
      <button
        id={`lead-${lead.id}`}
        type="button"
        aria-label={`${lead.company}: ${lead.role}, score ${scoreLabel(lead.score)}`}
        aria-current={selected ? "true" : undefined}
        onClick={() => selectLead(lead.id)}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 py-3 pr-3 pl-2 text-left"
      >
        <ScoreBadge score={lead.score} />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate font-semibold text-ink">{lead.role}</span>
          <span className="truncate text-[13px] text-muted">
            {lead.company} &middot; {metaLine(lead.location, null)}
          </span>
          {reasons !== "" && <span className="truncate text-small text-subtle">{reasons}</span>}
          {job && (
            <span className="flex items-center gap-1 text-small text-accent">
              <Spinner />
              {jobLabel(job)}
            </span>
          )}
        </span>
        <span className="flex shrink-0 flex-col items-end text-small text-subtle">
          {source && <span>{sourceName(source)}</span>}
          <span>{Number.isNaN(found.getTime()) ? "" : ago(now, found)}</span>
        </span>
      </button>
    </li>
  );
}

interface Props {
  shown: readonly Lead[];
  total: number;
  sources: readonly Source[];
  selectedId: number | undefined;
  checked: readonly number[];
  jobs: Readonly<Record<number, Job>>;
}

export function LeadList({ shown, total, sources, selectedId, checked, jobs }: Props) {
  const now = new Date();
  return (
    <div className="flex min-h-0 w-[380px] min-w-[260px] shrink flex-col gap-2.5">
      <div className="flex items-baseline gap-2">
        <Heading>New leads</Heading>
        <span className="text-muted">&middot; {total}</span>
      </div>
      <ul id="queue-list" role="list" aria-label="Leads" className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto">
        {shown.map((lead) => (
          <LeadCard
            key={lead.id}
            lead={lead}
            source={sources.find((s) => s.id === lead.sourceId)}
            selected={lead.id === selectedId}
            checked={checked.includes(lead.id)}
            anyChecked={checked.length > 0}
            job={jobs[lead.id]}
            now={now}
          />
        ))}
      </ul>
    </div>
  );
}
