// "Dismiss" on a job in CVs asks first, in a small dialog (the same as a lead's: an optional reason and
// "Don't show {company} again"); and what a dismissed job shows instead of its primary button: why and
// when, and a plain "Restore". Used by the application drawer and the full view.

import { RotateCcw } from "lucide-react";
import { useState } from "react";

import { Banner, Button, Checkbox, Dialog, TextInput } from "@/components/ui/kit";
import { FormError } from "@/components/ProblemHelp";
import { useErrorToast } from "@/hooks/useErrorToast";
import type { Action } from "@/hooks/useAsync";

import { dismissalLine } from "@/lib/applications/dismiss";
import type { ApplicationHeader, Dismissal, Dismissed, Restored } from "@/types/applications";

interface DialogProps {
  header: ApplicationHeader;
  open: boolean;
  dismiss: Action<[reason: string, excludeCompany: boolean], Dismissed>;
  onClose: () => void;
  /** The job was dismissed (a toast says what else went). */
  onDone: (out: Dismissed) => void;
}

/** Reason, "Don't show {company} again", Cancel and "Dismiss job". Why a dismissal failed stays in the
 *  dialog; a dismissal that went through closes it. */
export function DismissJobDialog({ header, open, dismiss, onClose, onDone }: DialogProps) {
  const [reason, setReason] = useState("");
  const [exclude, setExclude] = useState(false);
  const company = header.company.trim();
  const { clearError } = dismiss;
  /** Cancel, Esc or ✕: the next opening starts afresh. */
  const close = () => {
    clearError();
    setReason("");
    setExclude(false);
    onClose();
  };
  const confirm = () => {
    if (!dismiss.busy) {
      void dismiss.run(reason, exclude).then((out) => {
        if (out !== undefined) {
          setReason("");
          setExclude(false);
          onDone(out);
        }
      });
    }
  };
  return (
    <Dialog
      id="app-dismiss"
      open={open}
      busy={dismiss.busy}
      title="Dismiss this job"
      description={company === "" ? header.role : `${company} — ${header.role}`}
      width="sm"
      onClose={close}
      onSubmit={confirm}
      footer={
        <>
          <Button id="app-dismiss-cancel" variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button id="app-dismiss-confirm" type="submit" variant="primary" busy={dismiss.busy} disabled={dismiss.busy}>
            Dismiss job
          </Button>
        </>
      }
    >
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
      {dismiss.error && <FormError id="app-dismiss-error" error={dismiss.error} title="Not dismissed" />}
    </Dialog>
  );
}

/** A dismissed job: why and when, and a plain "Restore" (back at the status it had). A failed
 *  Restore is reported by a toast. */
export function DismissedNotice({
  dismissal,
  restore,
  onRestored,
}: {
  dismissal: Dismissal;
  restore: Action<[], Restored>;
  /** The job was restored (a toast says so). */
  onRestored?: (out: Restored) => void;
}) {
  const run = () => void restore.run().then((out) => out !== undefined && onRestored?.(out));
  useErrorToast(restore.error, { title: "The job could not be restored", onRetry: run, clear: restore.clearError });
  return (
    <Banner
      id="app-dismissed"
      tone="neutral"
      title="You dismissed this job"
      body={<span className="selectable">{dismissalLine(dismissal, new Date())}</span>}
      actions={
        <Button id="app-restore" size="compact" icon={RotateCcw} busy={restore.busy} onClick={run}>
          Restore
        </Button>
      }
    />
  );
}
