// Today, the start screen: what needs the user, then found jobs best match first, with the brief and
// the score breakdown, and the actions that turn a lead into an application (Prepare CV), drop it
// (Dismiss) or look at it (Open posting). At the top, a link box with "Add job" (and a file drop on
// the window) adds a job the user found themselves.
//
// The one primary button sits in the open lead's panel: "Prepare CV" while there are leads,
// "Search now" (in the empty state) when the list is empty. Everything else is a plain button.

import { Inbox, Search } from "lucide-react";

import { Button, Caption, EmptyState, ErrorBanner, Heading, Page, ProblemHelp, Spinner } from "@/shared";

import { lastSearchLine, sourceName, sourceNeedsAttention, sourceStatus, todayLabel } from "../domain/text";
import { reload, searchNow } from "../hooks/todayStore";
import { useToday } from "../hooks/useToday";
import { FilterBar } from "./FilterBar";
import { LeadList } from "./LeadList";
import { EmptyActions, LeadPanel, NoLeadOpen } from "./LeadPanel";
import { LinkBox } from "./LinkBox";
import { NeedsYou } from "./NeedsYou";

export function TodayScreen() {
  const t = useToday();
  const now = new Date();
  const total = t.items.length;
  const empty = total === 0;

  const searchLine = t.searchingLabel ? (
    <span className="flex items-center gap-2 text-body text-muted">
      <Spinner className="text-accent" />
      {t.searchingLabel}
    </span>
  ) : (
    <span className="text-body text-muted">{t.queue?.lastRun ? lastSearchLine(t.queue.lastRun, now) : "No search yet."}</span>
  );

  return (
    <Page
      label="Today"
      title={
        <div className="flex flex-col gap-0.5">
          <Caption>{todayLabel(now)}</Caption>
          <Heading level="h1">Today</Heading>
        </div>
      }
      subtitle={searchLine}
      // When the list is empty "Search now" is the primary button, in the empty state.
      actions={
        empty ? undefined : (
          <Button id="queue-search" icon={Search} busy={t.searchingLabel !== undefined} onClick={() => void searchNow()}>
            Search now
          </Button>
        )
      }
    >
      <div id="queue" className="flex flex-1 flex-col gap-5">
        <div className="flex flex-col gap-3">
          <LinkBox adding={t.adding} waiting={t.waiting.length} failures={t.addFailures} note={t.linkNote} />
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
          {t.loadError && <ErrorBanner id="queue-load-error" error={t.loadError} title="The job list could not be read" onRetry={reload} />}
        </div>
        <NeedsYou
          count={t.needsYou}
          waiting={t.waitingSources}
          failures={t.failureRows}
          prepared={t.prepared}
          opening={t.opening}
          continuing={t.continuing}
          openErrors={t.openErrors}
        />
        {!empty && <FilterBar filter={t.filter} sources={t.sources} shown={t.shown.length} total={total} />}
        {empty ? (
          <div id="queue-empty" role="group" aria-label="No jobs yet" className="flex flex-1 items-center justify-center">
            <div className="max-w-[520px]">
              <EmptyState
                icon={Inbox}
                title={t.loaded ? "No jobs yet" : "Loading the job list"}
                body="Paste a job link in the box above, or press Search now to look in your sources. Jobs appear here, best match first."
                action={<EmptyActions primary={t.primary} targets={t.targets} jobs={t.jobs} />}
              />
            </div>
          </div>
        ) : (
          <div className="flex min-h-[440px] flex-1 items-stretch gap-5">
            <LeadList shown={t.shown} total={total} sources={t.sources} selectedId={t.open?.id} checked={t.checked} jobs={t.jobs} />
            {t.open ? (
              <div className="flex min-h-0 min-w-[300px] flex-1 flex-col">
                <LeadPanel
                  key={t.open.id}
                  lead={t.open}
                  source={t.sourceOf(t.open)}
                  primary={t.primary}
                  targets={t.targets}
                  jobs={t.jobs}
                  profileSkills={t.profileSkills}
                />
              </div>
            ) : (
              <NoLeadOpen shown={t.shown.length} primary={t.primary} targets={t.targets} jobs={t.jobs} />
            )}
          </div>
        )}
      </div>
    </Page>
  );
}
