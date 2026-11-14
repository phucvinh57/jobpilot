import type { ReactNode } from "react";

/** A labelled row of the card: the label, an optional hint under it, then the control. It is a
 *  group, not a `<label>`: a row holds several buttons, and a label would forward clicks to the first. */
export function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div role="group" aria-label={label} className="flex flex-col gap-1.5">
      <span className="text-small font-medium text-muted">{label}</span>
      {hint && <span className="text-small text-subtle">{hint}</span>}
      {children}
    </div>
  );
}
