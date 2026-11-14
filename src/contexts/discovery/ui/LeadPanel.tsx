// The right pane for the open lead: title, score, the actions, why it fits, its skills, the company
// and the interview process. It scrolls on its own. The one primary button sits here ("Prepare CV"),
// or in the empty state ("Search now"); everything else is a plain button.

import clsx from "clsx";
import { ExternalLink, Search } from "lucide-react";
import { useState } from "react";

import {
  Button,
  Caption,
  Checkbox,
  CompanyBlock,
  EmptyState,
  InterviewBlock,
  LinkButton,
  openUrl,
  scoreLabel,
  scoreTone,
  TextInput,
  TONE_CLASS,
} from "@/shared";

import { actionLabel, type QueuePrimary } from "../domain/queue";
import { isWebLink, leadTitle, metaLine, sourceName } from "../domain/text";
import type { Job, Lead, Source } from "../domain/types";
import { useDismiss } from "../hooks/useDismiss";
import { prepare, searchNow } from "../hooks/todayStore";
import { Breakdown, SkillsBlock } from "./LeadBrief";

interface ActionProps {
  /** The open lead, if there is one: it gets "Open posting" and "Dismiss". */
  open?: Lead;
  primary: QueuePrimary;
  /** The leads "Prepare CV" acts on. */
  targets: readonly number[];
  jobs: Readonly<Record<number, Job>>;
  onDismiss?: () => void;
}

/** The primary button with its hint under it and, for the open lead, the plain "Open posting" and
 *  "Dismiss" buttons beside it. */
export function QueueActions({ open, primary, targets, jobs, onDismiss }: ActionProps) {
  const act = () => {
    if (primary.action === "prepareCv") {
      void prepare([...targets]);
    } else {
      void searchNow();
    }
  };
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button id="queue-primary" variant="primary" busy={primary.loading} disabled={!primary.enabled} onClick={act}>
          {actionLabel(primary.action)}
        </Button>
        {open && (
          <>
            <Button id="queue-open-posting" icon={ExternalLink} disabled={!isWebLink(open.url)} onClick={() => void openUrl(open.url).catch(() => undefined)}>
              Open posting
            </Button>
            <Button id="queue-dismiss-open" disabled={jobs[open.id] !== undefined} onClick={onDismiss}>
              Dismiss
            </Button>
          </>
        )}
      </div>
      {primary.hint && <Caption>{primary.hint}</Caption>}
    </div>
  );
}

/** The actions in the middle of an empty state. */
export function EmptyActions(props: Omit<ActionProps, "open" | "onDismiss">) {
  return (
    <div className="flex flex-col items-center">
      <QueueActions {...props} />
    </div>
  );
}

function DismissForm({ lead, onClose }: { lead: Lead; onClose: () => void }) {
  const [reason, setReason] = useState("");
  const [exclude, setExclude] = useState(false);
  const dismiss = useDismiss(lead.id);
  const company = lead.company.trim();
  return (
    <div id="queue-dismiss" role="group" aria-label="Dismiss this lead" className="flex flex-col gap-2.5 border border-border bg-surface-alt p-4">
      <p className="font-semibold text-ink">Dismiss {leadTitle(lead.company, lead.role)}</p>
      <TextInput id="queue-reason" aria-label="Why dismiss this lead" placeholder="Why? (optional)" value={reason} onChange={(e) => setReason(e.target.value)} />
      {company !== "" && (
        <Checkbox id="dismiss-exclude" label={`Don't show ${company} again`} checked={exclude} onChange={(e) => setExclude(e.target.checked)} />
      )}
      {dismiss.error && (
        <p role="alert" className="selectable text-body text-danger-fg">
          {dismiss.error.message}
        </p>
      )}
      <div className="flex gap-2">
        <Button
          id="dismiss-confirm"
          busy={dismiss.busy}
          disabled={dismiss.busy}
          onClick={() => void dismiss.run(reason, exclude).then((out) => out !== undefined && onClose())}
        >
          Dismiss lead
        </Button>
        <Button id="dismiss-cancel" variant="ghost" onClick={onClose}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

interface PanelProps {
  lead: Lead;
  source: Source | undefined;
  primary: QueuePrimary;
  targets: readonly number[];
  jobs: Readonly<Record<number, Job>>;
  profileSkills: readonly string[];
}

/** The open lead. Shown with `key={lead.id}`, so its form closes and it fades in whenever another
 *  lead is opened. */
export function LeadPanel({ lead, source, primary, targets, jobs, profileSkills }: PanelProps) {
  const [dismissing, setDismissing] = useState(false);
  const score = TONE_CLASS[scoreTone(lead.score)];
  const insight = lead.insight;
  const hasFit = lead.criteria !== null || (insight !== null && (insight.requiredSkills.length > 0 || insight.preferredSkills.length > 0));
  return (
    <div id="queue-lead" role="group" aria-label={`${lead.company}: ${lead.role}`} className="fade-in flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
      <div className="flex flex-col gap-5 border border-border bg-surface p-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex min-w-[240px] flex-1 flex-col gap-1">
            <p className="text-body text-muted">
              {lead.company} &middot; {metaLine(lead.location, source ? sourceName(source) : null)}
            </p>
            <h2 className="selectable font-heading text-[24px] leading-tight font-semibold text-ink">{lead.role}</h2>
            {isWebLink(lead.url) && (
              <LinkButton id="lead-url" aria-label={`Posting: ${lead.url}`} onClick={() => void openUrl(lead.url).catch(() => undefined)}>
                {lead.url}
              </LinkButton>
            )}
          </div>
          <div className="flex flex-col items-end">
            <span aria-label={`Fit score ${scoreLabel(lead.score)}`} className={clsx("font-heading text-[36px] leading-none font-semibold", score.fg)}>
              {scoreLabel(lead.score)}
            </span>
            <Caption>fit score</Caption>
          </div>
        </div>
        <QueueActions open={lead} primary={primary} targets={targets} jobs={jobs} onDismiss={() => setDismissing(true)} />
        {dismissing && <DismissForm lead={lead} onClose={() => setDismissing(false)} />}
        {hasFit && (
          <div className="flex flex-wrap items-start gap-6">
            <Breakdown criteria={lead.criteria} />
            <SkillsBlock skills={insight ? { required: insight.requiredSkills, preferred: insight.preferredSkills } : undefined} profileSkills={profileSkills} />
          </div>
        )}
        <div className="border-t border-border pt-5">
          {insight ? (
            <div className="flex flex-wrap items-start gap-6">
              <div className="min-w-[220px] flex-1">
                <CompanyBlock insight={insight} />
              </div>
              <div className="min-w-[220px] flex-1">
                <InterviewBlock insight={insight} />
              </div>
            </div>
          ) : (
            <p className="text-body text-muted">No company brief for this lead.</p>
          )}
        </div>
      </div>
    </div>
  );
}

/** What the right pane says when no lead is open. */
export function NoLeadOpen({ shown, primary, targets, jobs }: { shown: number; primary: QueuePrimary; targets: readonly number[]; jobs: Readonly<Record<number, Job>> }) {
  return (
    <div className="min-w-[300px] flex-1">
      <EmptyState
        icon={Search}
        title={shown === 0 ? "No lead matches the filters" : "Select a lead"}
        body={shown === 0 ? "Loosen a filter to see more jobs." : "Pick a job on the left to see why it matches."}
        action={<EmptyActions primary={primary} targets={targets} jobs={jobs} />}
      />
    </div>
  );
}
