// The drawer of the Leads stage: the open lead's score, the actions, why it fits, its skills, the
// company and the interview process. While it is open it holds the one primary button ("Prepare
// CV", for the ticked leads when there are some, else for this one); everything else is plain.

import clsx from "clsx";
import { ExternalLink } from "lucide-react";
import { useState } from "react";

import {
  Button,
  Caption,
  Checkbox,
  CompanyBlock,
  Drawer,
  InterviewBlock,
  LinkButton,
  openUrl,
  scoreLabel,
  scoreTone,
  TextInput,
  TONE_CLASS,
  useNav,
} from "@/shared";

import { actionLabel, type QueuePrimary } from "../domain/queue";
import { isWebLink, leadTitle, metaLine, sourceName } from "../domain/text";
import type { Job, Lead, Source } from "../domain/types";
import { useDismiss } from "../hooks/useDismiss";
import { prepare } from "../hooks/todayStore";
import { useToday } from "../hooks/useToday";
import { Breakdown, SkillsBlock } from "./LeadBrief";

interface ActionProps {
  open: Lead;
  primary: QueuePrimary;
  /** The leads "Prepare CV" acts on. */
  targets: readonly number[];
  jobs: Readonly<Record<number, Job>>;
  onDismiss: () => void;
}

/** "Prepare CV" (the primary button) with its hint under it, and the plain "Open posting" and
 *  "Dismiss" beside it. */
function QueueActions({ open, primary, targets, jobs, onDismiss }: ActionProps) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button id="queue-primary" variant="primary" busy={primary.loading} disabled={!primary.enabled} onClick={() => void prepare([...targets])}>
          {actionLabel(primary.action)}
        </Button>
        <Button id="queue-open-posting" icon={ExternalLink} disabled={!isWebLink(open.url)} onClick={() => void openUrl(open.url).catch(() => undefined)}>
          Open posting
        </Button>
        <Button id="queue-dismiss-open" disabled={jobs[open.id] !== undefined} onClick={onDismiss}>
          Dismiss
        </Button>
      </div>
      {primary.hint && <Caption>{primary.hint}</Caption>}
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

interface BodyProps {
  lead: Lead;
  source: Source | undefined;
  primary: QueuePrimary;
  targets: readonly number[];
  jobs: Readonly<Record<number, Job>>;
  profileSkills: readonly string[];
}

/** What the drawer shows for `lead`. Shown with `key={lead.id}`, so its form closes and it fades in
 *  whenever another lead is opened. */
function LeadBody({ lead, source, primary, targets, jobs, profileSkills }: BodyProps) {
  const [dismissing, setDismissing] = useState(false);
  const score = TONE_CLASS[scoreTone(lead.score)];
  const insight = lead.insight;
  const hasFit = lead.criteria !== null || (insight !== null && (insight.requiredSkills.length > 0 || insight.preferredSkills.length > 0));
  return (
    <div id="queue-lead" className="fade-in flex flex-col gap-5">
      <div className="flex items-start justify-between gap-4">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <p className="text-body text-muted">
            {lead.company} &middot; {metaLine(lead.location, source ? sourceName(source) : null)}
          </p>
          <p className="selectable font-heading text-h2 font-semibold text-ink">{lead.role}</p>
          {isWebLink(lead.url) && (
            <LinkButton id="lead-url" aria-label={`Posting: ${lead.url}`} onClick={() => void openUrl(lead.url).catch(() => undefined)}>
              {lead.url}
            </LinkButton>
          )}
        </div>
        <div className="flex shrink-0 flex-col items-end">
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
          <div className="flex flex-col gap-5">
            <CompanyBlock insight={insight} />
            <InterviewBlock insight={insight} />
          </div>
        ) : (
          <p className="text-body text-muted">No company brief for this lead.</p>
        )}
      </div>
    </div>
  );
}

/** The lead drawer (`#drawer`), named "{company}: {role}". Open while navigation asks for a lead
 *  that is listed; Esc or its ✕ closes it. */
export function LeadDrawer() {
  const t = useToday();
  const closeDrawer = useNav((s) => s.closeDrawer);
  const open = t.open;
  return (
    <Drawer open={open !== undefined} title={open ? `${open.company}: ${open.role}` : ""} width="md" onClose={closeDrawer} id="drawer">
      {open && (
        <LeadBody
          key={open.id}
          lead={open}
          source={t.sourceOf(open)}
          primary={t.primary}
          targets={t.targets}
          jobs={t.jobs}
          profileSkills={t.profileSkills}
        />
      )}
    </Drawer>
  );
}
