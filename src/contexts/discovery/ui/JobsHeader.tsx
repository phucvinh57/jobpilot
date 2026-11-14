// What the Leads stage puts in the Jobs page's header: the date with the last search, "Search now",
// and under the link box the job sources with how their last run went, and the note of the last
// search or dismissal.

import { Search } from "lucide-react";

import { Button, Caption, ProblemHelp, Spinner } from "@/shared";

import { lastSearchLine, sourceName, sourceNeedsAttention, sourceStatus, todayLabel } from "../domain/text";
import { searchNow } from "../hooks/todayStore";
import { useToday } from "../hooks/useToday";

/** `Saturday, 14 November · Last search 5 min ago: Found 3 new leads`, or the search running. */
export function LeadsHeaderLine() {
  const t = useToday();
  const now = new Date();
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
      <Caption>{todayLabel(now)}</Caption>
      <span aria-hidden className="text-subtle">
        &middot;
      </span>
      {t.searchingLabel ? (
        <span className="flex items-center gap-2 text-body text-muted">
          <Spinner className="text-accent" />
          {t.searchingLabel}
        </span>
      ) : (
        <span className="text-body text-muted">{t.queue?.lastRun ? lastSearchLine(t.queue.lastRun, now) : "No search yet."}</span>
      )}
    </div>
  );
}

/** "Search now" (`#queue-search`): the primary button when the list is empty and nothing else holds
 *  it, plain otherwise. `otherDrawerOpen`: another drawer of the page (an application's) is open
 *  and owns the primary. Off while a search runs, and when no job source is on. */
export function SearchNowButton({ otherDrawerOpen = false }: { otherDrawerOpen?: boolean }) {
  const t = useToday();
  const primary = t.placeFor(otherDrawerOpen) === "search";
  const noSource = t.loaded && !t.sources.some((s) => s.enabled);
  return (
    <Button
      id="queue-search"
      variant={primary ? "primary" : "secondary"}
      icon={Search}
      busy={t.searchingLabel !== undefined}
      disabled={noSource}
      title={noSource ? "No job source yet: add one in Settings, then search." : undefined}
      onClick={() => void searchNow()}
    >
      Search now
    </Button>
  );
}

/** The job sources and how each last run went (`#queue-sources`), then the note of the last search
 *  or dismissal (`#queue-note`) with what to do about a failure. Nothing when there is neither. */
export function SourcesLine() {
  const t = useToday();
  const now = new Date();
  if (t.sources.length === 0 && !t.note) {
    return null;
  }
  return (
    <div className="flex flex-col gap-1">
      {t.sources.length > 0 && (
        <ul id="queue-sources" role="list" aria-label="Job sources" className="flex flex-wrap gap-x-4 gap-y-0.5 text-small">
          {t.sources.map((s) => (
            <li key={s.id} id={`source-${s.id}`} className={sourceNeedsAttention(s) ? "text-warn-fg" : "text-subtle"}>
              {`${sourceName(s)}: ${sourceStatus(s, now)}${s.enabled ? "" : " (off)"}`}
            </li>
          ))}
        </ul>
      )}
      {t.note && (
        <div className="flex flex-col gap-1">
          <p id="queue-note" className={t.note.ok ? "selectable text-body text-muted" : "selectable text-body text-danger-fg"}>
            {t.note.text}
          </p>
          {!t.note.ok && t.note.problem && <ProblemHelp problem={t.note.problem} />}
        </div>
      )}
    </div>
  );
}
