// What the Leads stage puts in the Jobs page's header: the date with the last search, "Search now",
// and under the link box the job sources with how their last run went. How a search or a dismissal
// ended is a toast (the last search's result stays in the header line).

import { Search } from "lucide-react";

import { Button, Caption, Spinner } from "@/components/ui/kit";

import { lastSearchLine, sourceName, sourceNeedsAttention, sourceStatus, todayLabel } from "@/lib/discovery/text";
import { searchNow } from "@/stores/today";
import { useToday } from "@/hooks/discovery/useToday";

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
        <span className="text-body text-muted">
          {t.queue?.lastRun ? lastSearchLine(t.queue.lastRun, now) : "No search yet."}
        </span>
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

/** The job sources and how each last run went (`#queue-sources`). Nothing without a source. */
export function SourcesLine() {
  const t = useToday();
  const now = new Date();
  if (t.sources.length === 0) {
    return null;
  }
  return (
    <ul id="queue-sources" role="list" aria-label="Job sources" className="flex flex-wrap gap-x-4 gap-y-0.5 text-small">
      {t.sources.map((s) => (
        <li key={s.id} id={`source-${s.id}`} className={sourceNeedsAttention(s) ? "text-warn-fg" : "text-subtle"}>
          {`${sourceName(s)}: ${sourceStatus(s, now)}${s.enabled ? "" : " (off)"}`}
        </li>
      ))}
    </ul>
  );
}
