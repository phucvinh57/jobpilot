// The Leads stage of the Jobs page: the found jobs, best match first, as a full-width list. A row
// opens the lead in the drawer (`LeadDrawer`); ticked rows get a bar with "Prepare N CVs". An
// empty list says how to get jobs ("Search now" in the header is then the primary button).
// "Show: Dismissed" lists every dismissed job instead (`DismissedList`), each with Restore.

import { Inbox, X } from "lucide-react";

import { Button, Caption, EmptyState } from "@/components/ui/kit";
import { LoadError } from "@/components/ProblemHelp";

import { prepareManyLabel } from "@/lib/discovery/queue";
import { clearChecked, prepare, reload, reloadDismissed } from "@/stores/today";
import { useNav } from "@/stores/nav";
import { useToday, type TodayView } from "@/hooks/discovery/useToday";
import { DismissedList } from "./DismissedList";
import { FilterBar } from "./FilterBar";
import { LeadList } from "./LeadList";

/** Over the list while leads are ticked and the drawer is closed: "Prepare N CVs" (the primary
 *  button unless another drawer holds it) and a plain "Clear". */
function BulkBar({ t, primary }: { t: TodayView; primary: boolean }) {
  const n = t.targets.length;
  return (
    <div
      id="queue-bulk"
      role="group"
      aria-label={`${t.checked.length} selected`}
      className="flex flex-wrap items-center gap-3 border border-border bg-surface-alt px-3 py-2"
    >
      <span className="text-body font-semibold text-ink">{t.checked.length} selected</span>
      <Button
        id="queue-prepare-checked"
        variant={primary ? "primary" : "secondary"}
        size="compact"
        busy={t.primary.loading}
        disabled={!t.primary.enabled}
        onClick={() => void prepare([...t.targets])}
      >
        {prepareManyLabel(n)}
      </Button>
      <Button id="queue-clear-checked" variant="ghost" size="compact" icon={X} onClick={clearChecked}>
        Clear
      </Button>
      {t.primary.hint && <Caption>{t.primary.hint}</Caption>}
    </div>
  );
}

/** "Show: Dismissed": the filters (search) and every dismissed job, newest first. */
function DismissedView({ t }: { t: TodayView }) {
  const openAppId = useNav((s) => (s.drawer?.kind === "app" ? s.drawer.id : undefined));
  return (
    <>
      <FilterBar
        filter={t.filter}
        sources={t.sources}
        shown={t.dismissedShown.length}
        total={t.dismissedItems.length}
        dismissed={t.dismissedCount}
      />
      <LoadError
        id="dismissed-load-error"
        error={t.dismissedError}
        title="The dismissed jobs could not be read"
        onRetry={reloadDismissed}
      />
      <DismissedList
        shown={t.dismissedShown}
        restoring={t.restoring}
        openAppId={openAppId}
        empty={
          t.dismissed === undefined ? (
            "Loading the dismissed jobs"
          ) : t.dismissedItems.length === 0 ? (
            "No dismissed jobs. Jobs you dismiss, and the ones your preferences filter out, appear here."
          ) : (
            <span className="flex flex-col gap-1">
              <span className="font-semibold text-ink">No dismissed job matches the search</span>
              <span>Clear the search to see them all.</span>
            </span>
          )
        }
      />
    </>
  );
}

/** The stage's content: why the list could not be read, the filters, the bulk bar, then the list
 *  or the empty state. `otherDrawerOpen`: another drawer of the page owns the primary button. */
export function LeadsStage({ otherDrawerOpen = false }: { otherDrawerOpen?: boolean }) {
  const t = useToday();
  const total = t.items.length;
  const place = t.placeFor(otherDrawerOpen);
  const showBulk = t.open === undefined && t.checked.length > 0;
  // Search now is off without a source: the empty state says why.
  const why = !t.primary.enabled && t.primary.action === "searchNow" ? t.primary.hint : undefined;

  return (
    <div id="queue" className="flex flex-1 flex-col gap-3">
      <LoadError id="queue-load-error" error={t.loadError} title="The job list could not be read" onRetry={reload} />
      {t.filter.show === "dismissed" ? (
        <DismissedView t={t} />
      ) : total === 0 ? (
        <>
          {t.dismissedCount > 0 && (
            <FilterBar filter={t.filter} sources={t.sources} shown={0} total={0} dismissed={t.dismissedCount} />
          )}
          <div
            id="queue-empty"
            role="group"
            aria-label="No jobs yet"
            className="flex flex-1 items-center justify-center py-6"
          >
            <div className="max-w-[520px]">
              <EmptyState
                icon={Inbox}
                title={t.loaded ? "No jobs yet" : "Loading the job list"}
                body="Paste a job link in the box above, or press Search now to look in your sources. Jobs appear here, best match first."
                action={why && <Caption>{why}</Caption>}
              />
            </div>
          </div>
        </>
      ) : (
        <>
          <FilterBar
            filter={t.filter}
            sources={t.sources}
            shown={t.shown.length}
            total={total}
            dismissed={t.dismissedCount}
          />
          {showBulk && <BulkBar t={t} primary={place === "bulk"} />}
          <LeadList
            shown={t.shown}
            sources={t.sources}
            openId={t.open?.id}
            checked={t.checked}
            jobs={t.jobs}
            empty={
              <span className="flex flex-col gap-1">
                <span className="font-semibold text-ink">No lead matches the filters</span>
                <span>Loosen a filter to see more jobs.</span>
              </span>
            }
          />
        </>
      )}
    </div>
  );
}
