import { Spinner } from "@/components/ui/kit";

/** The step a running research is at (`#trends-step`): one on the screen at a time. */
export function RunStatus({ step, period }: { step: string; period?: string }) {
  return (
    <span className="flex items-center gap-2">
      <Spinner className="shrink-0 text-accent" />
      <span id="trends-step" role="status" className="text-body text-muted">
        {period ? `${period}: ${step}` : step}
      </span>
    </span>
  );
}
