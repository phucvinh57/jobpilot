import { Button, Caption, Dialog, Segmented } from "@/components/ui/kit";

import { CADENCES, collectCaption } from "@/lib/trends/format";
import type { Cadence } from "@/types/trends";

/** Start a research: pick the period (week, month, quarter, year), then "Collect & build digest".
 *  The run takes minutes, so the dialog does not wait for it: `onStart` starts it and the caller
 *  closes the dialog. */
export function NewResearchDialog({
  open,
  cadence,
  onCadence,
  periodLabel,
  onStart,
  onClose,
}: {
  open: boolean;
  cadence: Cadence;
  onCadence: (cadence: Cadence) => void;
  /** The period the run would cover (`2026-W46`); `undefined` until it has been read. */
  periodLabel: string | undefined;
  onStart: () => void;
  onClose: () => void;
}) {
  const ready = periodLabel !== undefined;
  return (
    <Dialog
      id="trends-new-dialog"
      open={open}
      title="New research"
      onClose={onClose}
      onSubmit={() => ready && onStart()}
      footer={
        <>
          <Button id="trends-new-cancel" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button id="trends-collect" type="submit" variant="primary" disabled={!ready}>
            Collect &amp; build digest
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-1">
        <span className="text-small font-medium text-muted">Period</span>
        <div id="trends-cadence">
          <Segmented label="Digest period" choices={CADENCES} value={cadence} onChange={onCadence} />
        </div>
      </div>
      <Caption>{ready ? collectCaption(periodLabel) : "Reading the period…"}</Caption>
    </Dialog>
  );
}
