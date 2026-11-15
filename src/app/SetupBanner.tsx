// Compact bar above the main pane while something the main flow needs is missing (no AI
// provider, no typst), and until the job preferences are saved. One line per missing thing, each
// with its action button. Nothing when everything is in place or the first check has not finished.

import { Settings2, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { Button, ProblemActions, useNav, useShell } from "@/shared";

function Line({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <div
      id={`${id}-banner`}
      role="group"
      aria-label={title}
      className="flex items-center gap-3 border border-l-2 border-warn/45 border-l-warn bg-warn-bg px-3 py-1.5"
    >
      <TriangleAlert aria-hidden className="size-4 shrink-0 text-warn" />
      <p className="min-w-0 flex-1 truncate text-small text-warn-fg">{title}</p>
      <div className="flex shrink-0 gap-2">{children}</div>
    </div>
  );
}

export function SetupBanner() {
  const status = useShell((s) => s.status);
  const screen = useNav((s) => s.screen);
  const openSettings = useNav((s) => s.openSettings);
  // Settings is where the user fixes these; no need to nag there.
  if (!status || screen === "settings") {
    return null;
  }
  if (status.blockers.length === 0 && status.prefsSaved) {
    return null;
  }
  return (
    <div className="mx-6 mt-4 flex flex-col gap-1">
      {status.blockers.map((blocker) => (
        <Line key={blocker.id} id={blocker.id} title={blocker.title}>
          <ProblemActions problem={blocker.problem} />
        </Line>
      ))}
      {!status.prefsSaved && (
        <Line id="setup-preferences" title="Tell JobPilot what job you are looking for.">
          <Button size="compact" icon={Settings2} onClick={() => openSettings("Profile & preferences")}>
            Set your preferences
          </Button>
        </Line>
      )}
    </div>
  );
}
