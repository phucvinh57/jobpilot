// The Application screen (the drawer's "Open full view"): a breadcrumb back to the Jobs page, the
// role, company and status, the ONE primary action (Generate -> Approve CV -> Review & Submit ->
// Submit) with a plain "Dismiss" while the job is in CVs, the steps, and the tabs JD / CV / Apply /
// Timeline. The shell decides the primary action; this view only shows it. A dismissed job shows why,
// with a plain "Restore", and no primary.

import { Inbox, Settings } from "lucide-react";
import { useState } from "react";

import { Button, Caption, EmptyState, Page, Spinner, Stepper, Tabs, Tag } from "@/components/ui/kit";
import { ErrorBanner } from "@/components/ProblemHelp";
import { sectionPlace, useNav } from "@/stores/nav";
import { useShell } from "@/stores/shell";

import { crumbFor } from "@/lib/applications/crumb";
import { TABS, type Tab } from "@/lib/applications/tabs";
import { useApplicationScreen } from "@/hooks/applications/useApplicationScreen";
import { useDismissJob } from "@/hooks/applications/useDismissJob";
import { jobDismissedNote, jobRestoredNote } from "@/lib/applications/dismiss";
import { ApplyTab } from "@/components/applications/ApplyTab";
import { CvTab } from "@/components/applications/CvTab";
import { DismissedNotice, DismissJobForm } from "@/components/applications/DismissJob";
import { JdTab } from "@/components/applications/JdTab";
import { TimelineTab } from "@/components/applications/TimelineTab";

function Nothing({ none }: { none: boolean }) {
  return (
    <main aria-label="Application" className="mx-auto flex h-full max-w-[480px] items-center justify-center p-6">
      <EmptyState
        icon={Inbox}
        title={none ? "No applications yet" : "Nothing selected"}
        body={
          none
            ? "Open Jobs and paste the link of a job posting (or drop a .md, .txt, .html or .pdf file on the window). Press Prepare CV on the job: JobPilot writes a CV tailored to it from your profile, then helps you apply."
            : "Open an application from the Jobs page, or prepare a CV for one of its Leads."
        }
      />
    </main>
  );
}

/** Back to the Jobs page as the user left it (its stage and its drawer). */
function Breadcrumb({ company }: { company: string }) {
  const show = useNav((s) => s.show);
  const stage = useNav((s) => s.stage);
  const crumb = crumbFor(stage);
  return (
    <div className="flex min-w-0 items-center gap-1.5 text-small text-muted">
      <button
        id={crumb.id}
        type="button"
        aria-label={crumb.aria}
        onClick={() => show("jobs")}
        className="cursor-pointer text-accent hover:underline"
      >
        {crumb.label}
      </button>
      <span aria-hidden>/</span>
      <span className="min-w-0 truncate">{company}</span>
    </div>
  );
}

function OpenApplication({ id }: { id: number }) {
  const model = useApplicationScreen(id);
  const openSection = useNav((s) => s.openSection);
  const dismissJob = useDismissJob(id);
  const [dismissing, setDismissing] = useState(false);
  const [dismissNote, setDismissNote] = useState<string>();
  const [restoreNote, setRestoreNote] = useState<string>();
  const { head, apply, tab } = model;
  const header = head.header;

  if (header === undefined) {
    return (
      <Page label="Application" title="Application">
        {head.loadError ? (
          <ErrorBanner error={head.loadError} title="The application could not be read" />
        ) : (
          <Caption>Loading...</Caption>
        )}
      </Page>
    );
  }
  if (header === null) {
    return <NothingOpen />;
  }

  const primary = head.primary;
  const showNextJob = header.showNextJob && primary === null;
  const canDismiss = header.dismissible && header.dismissed === null;
  return (
    <Page
      label="Application"
      title={
        <div className="flex min-w-0 flex-col gap-1">
          <Breadcrumb company={header.company} />
          <h1 className="font-heading text-h1 font-semibold text-ink">{header.role}</h1>
        </div>
      }
      subtitle={
        <div className="flex flex-wrap items-center gap-2">
          <span>{header.subtitle}</span>
          <Tag tone={header.status.tone}>{header.status.label}</Tag>
          {head.jobLabel !== null && (
            <span className="flex items-center gap-2 text-small text-muted">
              <Spinner className="size-3.5 text-accent" />
              {head.jobLabel}
            </span>
          )}
        </div>
      }
      actions={
        <>
          {showNextJob && (
            <Button id="next-job" icon={Inbox} onClick={model.nextJob}>
              Next job
            </Button>
          )}
          {canDismiss && (
            <Button id="app-dismiss-open" disabled={dismissing} onClick={() => setDismissing(true)}>
              Dismiss
            </Button>
          )}
          {primary?.settings != null && (
            <Button id="primary-settings" icon={Settings} onClick={() => openSection(primary.settings ?? "")}>
              Open {sectionPlace(primary.settings)} &gt; {primary.settings}
            </Button>
          )}
          {primary !== null && (
            <Button
              id="primary-action"
              variant="primary"
              busy={primary.loading}
              disabled={!primary.enabled}
              onClick={model.onPrimary}
            >
              {primary.label}
            </Button>
          )}
        </>
      }
    >
      {primary?.hint != null && <p className="text-right text-small text-muted">{primary.hint}</p>}
      {dismissing && canDismiss && (
        <DismissJobForm
          header={header}
          dismiss={dismissJob.dismiss}
          onCancel={() => setDismissing(false)}
          onDone={(out) => {
            setDismissing(false);
            setRestoreNote(undefined);
            setDismissNote(jobDismissedNote(out));
          }}
        />
      )}
      {header.dismissed !== null && (
        <DismissedNotice
          dismissal={header.dismissed}
          restore={dismissJob.restore}
          onRestored={(out) => {
            setDismissNote(undefined);
            setRestoreNote(jobRestoredNote(out));
          }}
        />
      )}
      {restoreNote !== undefined && header.dismissed === null && (
        <p role="status" className="text-body text-muted">
          {restoreNote}
        </p>
      )}
      {dismissNote !== undefined && header.dismissed !== null && (
        <p role="status" className="text-body text-muted">
          {dismissNote}
        </p>
      )}
      {head.error !== null && (
        <ErrorBanner
          id="job-error"
          error={head.error}
          title="The last job failed"
          onRetry={head.retry}
          onDismiss={head.dismiss}
        />
      )}
      <Stepper steps={header.steps} current={header.stepIndex} />
      <Tabs<Tab> label="Application sections" tabs={TABS} value={tab} onChange={model.setTab} />
      {tab === "jd" && <JdTab id={id} />}
      {tab === "cv" && (
        <CvTab
          id={id}
          cvEpoch={head.cvEpoch}
          busy={head.jobLabel !== null}
          readOnly={header.dismissed !== null}
          revise={head.revise}
        />
      )}
      {tab === "apply" && <ApplyTab id={id} session={apply} />}
      {tab === "timeline" && <TimelineTab id={id} />}
    </Page>
  );
}

/** The application that was open is gone (deleted): the same view as having none open. */
function NothingOpen() {
  const count = useShell((s) => s.status?.applications);
  return <Nothing none={count === 0} />;
}

export function ApplicationPage() {
  const id = useNav((s) => s.applicationId);
  if (id === undefined) {
    return <NothingOpen />;
  }
  // Keyed by id: opening another application starts every tab from scratch.
  return <OpenApplication key={id} id={id} />;
}
