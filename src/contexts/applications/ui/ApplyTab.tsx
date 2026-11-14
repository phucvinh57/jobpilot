// The Apply tab: where the application goes, then review -> confirm -> submit. The form is opened and
// filled by a managed browser; the user reads every answer first and nothing is sent until they press
// Submit (the primary button of the screen). A form that stops (login, captcha, a field the run cannot
// fill) is handed to the user in a visible browser.

import clsx from "clsx";
import { Check } from "lucide-react";
import { useState } from "react";

import { Banner, Button, Caption, Card, Checkbox, ErrorBanner, Heading, Section, Spinner, Tag, TextInput, TONE_CLASS } from "@/shared";

import type { Apply, ApplyDone, ApplyHandoff, ApplyIdle, ApplyWorking } from "../domain/types";
import type { ApplySession } from "../hooks/useApplySession";
import { ReviewPanel } from "./ReviewPanel";
import { Screenshots } from "./Screenshots";

const READY_TEXT =
  "Press Review & Submit (top right) to open the form and fill it in. You review every answer first; nothing is sent until you press Submit.";

function ApplyError({ apply, session }: { apply: Apply; session: ApplySession }) {
  if (apply.error === null) {
    return null;
  }
  // Retry only re-opens and re-fills the form (never submits); a failed submit is retried with the
  // Submit button itself.
  return <ErrorBanner id="apply-error" error={apply.error} title="The application step failed" onRetry={apply.canRetry ? session.prepare : undefined} />;
}

function UrlLine({ idle, session }: { idle: ApplyIdle; session: ApplySession }) {
  const [url, setUrl] = useState("");
  if (idle.applyUrl !== null) {
    return (
      <div className="flex flex-col">
        <p className="selectable truncate text-body text-ink">{idle.applyUrl}</p>
        {idle.urlNote !== null && <Caption>{idle.urlNote}</Caption>}
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      <p className="text-body text-warn-fg">No apply URL found in the job description.</p>
      <div className="flex gap-2">
        <div className="min-w-0 flex-1">
          <TextInput aria-label="Apply URL" placeholder="Link of the application form, https://..." value={url} onChange={(e) => setUrl(e.target.value)} />
        </div>
        <Button id="save-apply-url" disabled={session.busy} onClick={() => session.saveUrl(url)}>
          Save
        </Button>
      </div>
    </div>
  );
}

function Idle({ id, idle, apply, session }: { id: number; idle: ApplyIdle; apply: Apply; session: ApplySession }) {
  return (
    <div className="flex flex-col gap-3">
      <Section title={idle.ats} hint="Where the application will be sent" actions={<Tag tone="neutral">detected</Tag>}>
        <UrlLine idle={idle} session={session} />
        <p className={clsx("text-body", TONE_CLASS[idle.sessionTone].fg)}>{idle.session}</p>
      </Section>
      <p className="text-body text-muted">{idle.applied ? "This application was sent. Replies are tracked from your email." : READY_TEXT}</p>
      {!idle.applied && (
        <>
          <Checkbox
            id="dry-run"
            label="Dry run: fill the form but never submit"
            checked={idle.dryRun}
            disabled={idle.dryRunForced}
            onChange={(e) => session.setDryRun(e.target.checked)}
          />
          {idle.dryRunForced && <Caption>JOBPILOT_DRY_RUN is set: submitting is disabled.</Caption>}
        </>
      )}
      {idle.hasProof && <Screenshots id={id} kind="proof" count={1} version={apply.generation} title="Confirmation" />}
      <ApplyError apply={apply} session={session} />
    </div>
  );
}

function Working({ working, lines }: { working: ApplyWorking; lines: string[] }) {
  const last = lines.length - 1;
  return (
    <Card>
      <div className="flex items-center gap-2 text-accent">
        <Spinner />
        <Heading>{working.title}</Heading>
      </div>
      <ul aria-label="Progress" className="flex flex-col gap-1">
        {lines.map((line, i) => (
          <li key={`${i}-${line}`} className={clsx("flex items-center gap-2 text-body", i === last ? "text-ink" : "text-muted")}>
            {i === last ? <span aria-hidden className="w-4 text-center">›</span> : <Check aria-hidden className="size-4" />}
            {line}
          </li>
        ))}
      </ul>
      <Caption>A headless browser is working on the form. This can take up to a minute.</Caption>
    </Card>
  );
}

function Done({ id, done, generation }: { id: number; done: ApplyDone; generation: number }) {
  return (
    <div className="flex flex-col gap-3">
      <Banner tone="ok" title="Application submitted" body={done.body} />
      {done.hasProof && <Screenshots id={id} kind="done" count={1} version={generation} title="Proof" />}
    </div>
  );
}

function Handoff({ id, handoff, apply, session }: { id: number; handoff: ApplyHandoff; apply: Apply; session: ApplySession }) {
  const opening = apply.busy === "Opening the browser...";
  const busy = apply.busy !== null || session.busy;
  return (
    <div className="flex flex-col gap-3">
      <Banner tone="warn" title={handoff.title} body={handoff.reason} />
      <p className="text-body text-muted">
        {handoff.headed
          ? "The visible browser is open at the form (your earlier answers were filled in again where possible). Finish and submit there, then mark the result below."
          : "Open a visible browser on the same profile to finish by hand, then mark the result."}
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Button id="open-visible" busy={opening} disabled={busy} onClick={session.openVisible}>
          {handoff.headed ? "Reopen visible browser" : "Open visible browser"}
        </Button>
        <Button id="mark-applied" disabled={busy} onClick={() => session.mark(true)}>
          Mark as applied
        </Button>
        <Button id="mark-failed" disabled={busy} onClick={() => session.mark(false)}>
          Mark failed
        </Button>
        {apply.busy !== null && <Caption>{apply.busy}</Caption>}
      </div>
      <ApplyError apply={apply} session={session} />
      {handoff.hasScreenshot && <Screenshots id={id} kind="handoff" count={1} version={apply.generation} title="What the page looked like" />}
    </div>
  );
}

export function ApplyTab({ id, session }: { id: number; session: ApplySession }) {
  const apply = session.apply;
  if (apply === undefined) {
    return session.loadError ? <ErrorBanner error={session.loadError} title="The apply step could not be read" /> : null;
  }
  return (
    <div id="apply-tab" className="flex flex-col gap-3">
      {apply.phase === "idle" && apply.idle !== null && <Idle id={id} idle={apply.idle} apply={apply} session={session} />}
      {(apply.phase === "preparing" || apply.phase === "submitting") && apply.working !== null && (
        <Working working={apply.working} lines={session.lines} />
      )}
      {apply.phase === "review" && apply.review !== null && <ReviewPanel id={id} review={apply.review} session={session} generation={apply.generation} />}
      {apply.phase === "done" && apply.done !== null && <Done id={id} done={apply.done} generation={apply.generation} />}
      {apply.phase === "handoff" && apply.handoff !== null && <Handoff id={id} handoff={apply.handoff} apply={apply} session={session} />}
    </div>
  );
}
