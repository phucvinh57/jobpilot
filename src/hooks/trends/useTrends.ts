// The Trends screen's use-cases: read what it shows, start a run (collect, refresh, continue,
// retry), pick the period, pick an earlier digest, add a topic to the preferences.

import { useCallback, useState } from "react";

import { useGraphProgress } from "@/hooks/useGraphProgress";
import { useQuery } from "@/hooks/useAsync";

import * as api from "@/api/trends";
import { readFailure } from "@/lib/trends/format";
import type { Job, JobKind } from "@/lib/trends/jobs";
import type { Cadence, Overview } from "@/types/trends";
import { useRuns, type Notice } from "./runs";
import { usePreferenceAdds } from "./usePreferenceAdds";

/** What was read, and for which cadence (until the answer for a new one lands, its period is not known). */
interface Loaded extends Overview {
  cadence: Cadence;
}

export function useTrends() {
  const [cadence, setCadence] = useState<Cadence>("weekly");
  // `undefined`: the newest digest.
  const [selected, setSelected] = useState<number>();
  const [showHistory, setShowHistory] = useState(false);

  const overview = useQuery<Loaded>(
    () => api.overview(selected, cadence).then((o) => ({ ...o, cadence })),
    [selected, cadence],
    // Sources and digests of this screen; the preferences decide which topics read "Added".
    ["trends", "profile"],
  );
  const data = overview.data;

  const running = useRuns((s) => s.running);
  const step = useRuns((s) => s.step);
  const stored = useRuns((s) => s.notice);
  const say = useRuns((s) => s.say);
  useGraphProgress(useRuns.getState().follow);

  // The read failed: say so, unless a run has something more recent to tell.
  const notice: Notice | undefined =
    stored ?? (overview.error ? { kind: "info", text: readFailure(overview.error.message), details: [] } : undefined);

  const adds = usePreferenceAdds(data, say);

  const start = useCallback(async (job: Job) => {
    const end = await useRuns.getState().start(job);
    if (end?.reportId != null) {
      setSelected(end.reportId);
    }
  }, []);

  // The period of the shown cadence, once it has been read (the run commands take it).
  const period = data?.cadence === cadence ? data.period.label : undefined;
  const begin = (kind: JobKind) => (period === undefined ? Promise.resolve() : start({ kind, period }));

  return {
    data,
    loading: overview.loading,
    cadence,
    /** A run in progress keeps its period: the choice waits for it to finish. */
    chooseCadence: (next: Cadence) => {
      if (!running) {
        setCadence(next);
      }
    },
    busy: running !== undefined,
    /** Not known yet: nothing to run for. */
    ready: period !== undefined,
    step,
    notice,
    collect: () => begin("collect"),
    regenerate: () => begin("regenerate"),
    /** "Continue" a collection paused at a login, captcha or block page. */
    resume: (periodLabel: string) => start({ kind: "resume", period: periodLabel }),
    /** "Build digest without it": the digest alone, from what is stored. */
    regenerateFor: (periodLabel: string) => start({ kind: "regenerate", period: periodLabel }),
    retry: (job: Job) => start(job),
    shownReport: data?.digest?.reportId,
    selectReport: setSelected,
    showHistory,
    toggleHistory: () => setShowHistory((open) => !open),
    adds,
  };
}
