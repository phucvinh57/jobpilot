// "Dismiss" on a job in CVs (the same form as a lead's: an optional reason and "Don't show {company}
// again"), and what a dismissed job shows instead of its primary button: why and when, and a plain
// "Restore". Used by the application drawer and the full view.

import { RotateCcw } from "lucide-react";
import { useState } from "react";

import { Banner, Button, Checkbox, TextInput } from "@/components/ui/kit";
import { ErrorBanner } from "@/components/ProblemHelp";
import type { Action } from "@/hooks/useAsync";

import { dismissalLine } from "@/lib/applications/dismiss";
import type { ApplicationHeader, Dismissal, Dismissed, Restored } from "@/types/applications";

interface FormProps {
  header: ApplicationHeader;
  dismiss: Action<[reason: string, excludeCompany: boolean], Dismissed>;
  onCancel: () => void;
  /** The job was dismissed (`out` says what else went). */
  onDone: (out: Dismissed) => void;
}

/** Reason, "Don't show {company} again", "Dismiss job" (plain: the primary stays the job's own) and Cancel. */
export function DismissJobForm({ header, dismiss, onCancel, onDone }: FormProps) {
  const [reason, setReason] = useState("");
  const [exclude, setExclude] = useState(false);
  const company = header.company.trim();
  const title = company === "" ? header.role : `${company} — ${header.role}`;
  return (
    <div
      id="app-dismiss"
      role="group"
      aria-label="Dismiss this job"
      className="flex flex-col gap-2.5 border border-border bg-surface-alt p-4"
    >
      <p className="font-semibold text-ink">Dismiss {title}</p>
      <TextInput
        id="app-dismiss-reason"
        aria-label="Why dismiss this job"
        placeholder="Why? (optional)"
        value={reason}
        onChange={(e) => setReason(e.target.value)}
      />
      {company !== "" && (
        <Checkbox
          id="app-dismiss-exclude"
          label={`Don't show ${company} again`}
          checked={exclude}
          onChange={(e) => setExclude(e.target.checked)}
        />
      )}
      {dismiss.error && (
        <p role="alert" className="selectable text-body text-danger-fg">
          {dismiss.error.message}
        </p>
      )}
      <div className="flex gap-2">
        <Button
          id="app-dismiss-confirm"
          busy={dismiss.busy}
          disabled={dismiss.busy}
          onClick={() => void dismiss.run(reason, exclude).then((out) => out !== undefined && onDone(out))}
        >
          Dismiss job
        </Button>
        <Button id="app-dismiss-cancel" variant="ghost" disabled={dismiss.busy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** A dismissed job: why and when, and a plain "Restore" (back at the status it had). */
export function DismissedNotice({
  dismissal,
  restore,
  onRestored,
}: {
  dismissal: Dismissal;
  restore: Action<[], Restored>;
  /** The job was restored (`out` says whether its company is still excluded). */
  onRestored: (out: Restored) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Banner
        id="app-dismissed"
        tone="neutral"
        title="You dismissed this job"
        body={<span className="selectable">{dismissalLine(dismissal, new Date())}</span>}
        actions={
          <Button
            id="app-restore"
            size="compact"
            icon={RotateCcw}
            busy={restore.busy}
            onClick={() => void restore.run().then((out) => out !== undefined && onRestored(out))}
          >
            Restore
          </Button>
        }
      />
      {restore.error && (
        <ErrorBanner error={restore.error} title="The job could not be restored" onDismiss={restore.clearError} />
      )}
    </div>
  );
}
