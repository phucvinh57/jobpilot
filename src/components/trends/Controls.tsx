import { Button, Caption, Segmented, Spinner } from "@/components/ui/kit";

import { CADENCES, collectCaption } from "@/lib/trends/format";
import type { Cadence } from "@/types/trends";

/** The period of a run (week, month, quarter, year), and the one primary button of the screen. */
export function Controls({
  cadence,
  onCadence,
  periodLabel,
  busy,
  ready,
  step,
  onCollect,
}: {
  cadence: Cadence;
  onCadence: (cadence: Cadence) => void;
  /** The period a run would cover (`2026-W46`); `undefined` until it has been read. */
  periodLabel: string | undefined;
  busy: boolean;
  /** The period is known, so a run can start. */
  ready: boolean;
  /** The step a running job is at. */
  step: string | undefined;
  onCollect: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-3">
        <div id="trends-cadence">
          <Segmented label="Digest period" choices={CADENCES} value={cadence} onChange={onCadence} disabled={busy} />
        </div>
        <Button id="trends-collect" variant="primary" busy={busy} disabled={!ready} onClick={onCollect}>
          Collect &amp; build digest
        </Button>
        {busy && step && (
          <span className="flex items-center gap-2">
            <Spinner className="text-accent" />
            <span id="trends-step" role="status" className="text-body text-muted">
              {step}
            </span>
          </span>
        )}
      </div>
      {periodLabel && <Caption>{collectCaption(periodLabel)}</Caption>}
    </div>
  );
}
