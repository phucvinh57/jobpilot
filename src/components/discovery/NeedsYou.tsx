// "Needs you": sources paused at a login, captcha or block page, CVs that could not be written, and
// CVs that are ready to read. Nothing here is solved by the app: a site that needs the user is dealt
// with in the visible browser, and the app only continues afterwards.

import { ExternalLink, X } from "lucide-react";
import type { ReactNode } from "react";

import { Banner, Button } from "@/components/ui/kit";
import { ProblemHelp } from "@/components/ProblemHelp";
import { useNav } from "@/stores/nav";

import type { FailureRow } from "@/lib/discovery/queue";
import { shorten } from "@/lib/discovery/text";
import type { Waiting } from "@/types/discovery";
import {
  closePrepared,
  continueSource,
  forgetFailure,
  openBrowser,
  prepare,
  skipSource,
  type Prepared,
} from "@/stores/today";
import { useToday } from "@/hooks/discovery/useToday";

/** Characters of a failure message the banner shows; the whole text goes under it. */
const FAILURE_SUMMARY_CHARS = 140;

/** A "Needs you" banner is at least this wide, so two fit side by side on a wide window. */
const BANNER = "flex min-w-[360px] flex-1 flex-col gap-1";

function what(reason: Waiting["reason"]): string {
  switch (reason) {
    case "login":
      return "needs you to sign in";
    case "captcha":
      return "shows a captcha";
    case "blocked":
      return "blocks automated access";
    default:
      return "is waiting for you";
  }
}

/** One source waiting for the user: Open browser (a visible window on the page), Continue and Skip.
 *  When the window could not be opened, a toast says why and what to do about it. */
function WaitingBanner({ waiting, opening, continuing }: { waiting: Waiting; opening: boolean; continuing: boolean }) {
  const { thread, sourceId: id } = waiting;
  const title = `${waiting.sourceName} ${what(waiting.reason)}`;
  const body =
    waiting.url !== null ? shorten(waiting.url, 90) : "Deal with the page in the browser, then press Continue.";
  return (
    <div id={`waiting-${id}`} className={BANNER}>
      <Banner
        tone="warn"
        title={title}
        body={body}
        actions={
          <>
            <Button
              id={`waiting-open-${id}`}
              size="compact"
              icon={ExternalLink}
              busy={opening}
              disabled={opening || continuing}
              onClick={() => void openBrowser(thread)}
            >
              Open browser
            </Button>
            <Button
              id={`waiting-continue-${id}`}
              size="compact"
              busy={continuing}
              disabled={continuing}
              onClick={() => void continueSource(thread)}
            >
              Continue
            </Button>
            <Button
              id={`waiting-skip-${id}`}
              size="compact"
              variant="ghost"
              disabled={continuing}
              onClick={() => void skipSource(thread)}
            >
              Skip
            </Button>
          </>
        }
      />
    </div>
  );
}

/** A CV that could not be written: the first line of the message (the whole text under it when there
 *  is more), what to do about it, Retry. It stays until it is retried or forgotten, so it is an item
 *  of "Needs you" (with the warn mark of the others), not an error message in red. */
function FailureBanner({ row }: { row: FailureRow }) {
  const { leadId: id, applicationId, problem } = row;
  const openAppDrawer = useNav((s) => s.openAppDrawer);
  const title = `The CV for ${row.title} could not be written`;
  const lines = row.message.split("\n");
  const firstLine = (lines[0] ?? "").trim();
  const hasMore = lines.length > 1 || Array.from(firstLine).length > FAILURE_SUMMARY_CHARS;
  const again = () => void prepare([id]);
  return (
    <div id={`failure-${id}`} className={BANNER}>
      <Banner
        tone="warn"
        title={title}
        body={shorten(firstLine, FAILURE_SUMMARY_CHARS)}
        actions={
          <>
            {!problem.retryable && (
              <Button id={`prepare-retry-${id}`} size="compact" onClick={again}>
                Try again
              </Button>
            )}
            {applicationId !== null && (
              <Button id={`prepare-open-${id}`} size="compact" onClick={() => openAppDrawer(applicationId)}>
                Open application
              </Button>
            )}
            <Button id={`prepare-forget-${id}`} size="compact" variant="ghost" onClick={() => void forgetFailure(id)}>
              Forget
            </Button>
          </>
        }
      />
      {hasMore && (
        <pre
          id={`failure-text-${id}`}
          className="selectable max-h-[72px] overflow-y-auto font-mono text-small whitespace-pre-wrap text-muted"
        >
          {row.message}
        </pre>
      )}
      <ProblemHelp problem={problem} onRetry={again} />
    </div>
  );
}

/** A CV that was written, as a banner that points at the application: read it, then approve it. */
function PreparedBanner({ index, prepared }: { index: number; prepared: Prepared }) {
  const openAppDrawer = useNav((s) => s.openAppDrawer);
  return (
    <div id={`prepared-${index}`} className={BANNER}>
      <Banner
        tone="ok"
        title={`CV ready for ${prepared.label}`}
        body="Read it, then approve it."
        actions={
          <>
            <Button id={`prepared-open-${index}`} size="compact" onClick={() => openAppDrawer(prepared.applicationId)}>
              Open application
            </Button>
            <Button
              id={`prepared-close-${index}`}
              size="compact"
              variant="ghost"
              icon={X}
              aria-label="Dismiss this message"
              onClick={() => closePrepared(index)}
            />
          </>
        }
      />
    </div>
  );
}

interface Props {
  count: number;
  waiting: readonly Waiting[];
  failures: readonly FailureRow[];
  prepared: readonly Prepared[];
  opening: readonly string[];
  continuing: readonly string[];
}

/** The banners, from what the store holds. */
function NeedsYouBanners({ count, waiting, failures, prepared, opening, continuing }: Props): ReactNode {
  if (waiting.length + failures.length + prepared.length === 0) {
    return null;
  }
  return (
    <div id="queue-needs-you" role="group" aria-label="Needs you" className="flex flex-col gap-2.5">
      <p className="text-small font-semibold text-warn-fg">NEEDS YOU &middot; {count}</p>
      <div className="flex flex-wrap gap-3">
        {waiting.map((w) => (
          <WaitingBanner
            key={w.thread}
            waiting={w}
            opening={opening.includes(w.thread)}
            continuing={continuing.includes(w.thread)}
          />
        ))}
        {failures.map((row) => (
          <FailureBanner key={row.leadId} row={row} />
        ))}
        {prepared.map((p, i) => (
          <PreparedBanner key={`${p.applicationId}-${i}`} index={i} prepared={p} />
        ))}
      </div>
    </div>
  );
}

/** The "Needs you" block (`#queue-needs-you`): sources waiting at a login, CVs that failed and CVs
 *  ready to read ("Open application" shows it in the drawer, at the CV stage). Nothing when there is
 *  nothing. */
export function NeedsYou(): ReactNode {
  const t = useToday();
  return (
    <NeedsYouBanners
      count={t.needsYou}
      waiting={t.waitingSources}
      failures={t.failureRows}
      prepared={t.prepared}
      opening={t.opening}
      continuing={t.continuing}
    />
  );
}
