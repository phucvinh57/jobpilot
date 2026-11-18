// The Trends screen's use-cases: read what it shows, start a run (collect, refresh, continue,
// retry, rerun a saved report), pick the period, show, export or delete a saved report, add a topic
// to the preferences.

import { useCallback, useState } from "react";

import { useErrorToast } from "@/hooks/useErrorToast";
import { useGraphProgress } from "@/hooks/useGraphProgress";
import { useAction, useQuery } from "@/hooks/useAsync";
import { toast } from "@/stores/toast";

import * as api from "@/api/trends";
import { readFailure, reportFileName } from "@/lib/trends/format";
import type { Job, JobKind } from "@/lib/trends/jobs";
import type { Cadence, Overview } from "@/types/trends";
import { useRuns, type InfoNotice } from "./runs";
import { usePreferenceAdds } from "./usePreferenceAdds";

/** What was read, and for which cadence (until the answer for a new one lands, its period is not known). */
interface Loaded extends Overview {
  cadence: Cadence;
}

export function useTrends() {
  const [cadence, setCadence] = useState<Cadence>("weekly");
  // `undefined`: the newest digest.
  const [selected, setSelected] = useState<number>();

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
  const clearFailure = useRuns((s) => s.clearFailure);
  useGraphProgress(useRuns.getState().follow);

  const start = useCallback(async (job: Job) => {
    const end = await useRuns.getState().start(job);
    if (end?.reportId != null) {
      setSelected(end.reportId);
    }
  }, []);

  // A run that did not finish is a toast with Retry (carrying on where it stopped), shown once:
  // then the store forgets it. One that failed while the screen was closed toasts on return.
  const failed = stored?.kind === "error" ? stored : undefined;
  useErrorToast(failed?.error, {
    title: "The run did not finish",
    onRetry: failed && (() => void start(failed.retry)),
    clear: clearFailure,
  });

  // The read failed: say so, unless a run has something more recent to tell.
  const notice: InfoNotice | undefined =
    stored?.kind === "info"
      ? stored
      : overview.error
        ? { kind: "info", text: readFailure(overview.error.message), details: [] }
        : undefined;

  const adds = usePreferenceAdds(data, say);

  const exportAction = useAction(async (id: number, period: string) => {
    const fileName = reportFileName(period);
    if (await api.exportPdf(id, fileName)) {
      toast.ok("Saved PDF", fileName);
    }
  });
  useErrorToast(exportAction.error, { title: "The report could not be saved as PDF", clear: exportAction.clearError });

  const shownReport = data?.digest?.reportId;
  // The period of the report on screen (what its PDF is named after).
  const shownPeriod = data?.history.find((r) => r.id === shownReport)?.period;
  const deleteAction = useAction(async (id: number) => {
    await api.deleteReport(id);
    // The report on screen is gone: show the newest.
    if (id === shownReport || id === selected) {
      setSelected(undefined);
    }
    toast.ok("Report deleted");
    return true;
  });
  useErrorToast(deleteAction.error, { title: "The report could not be deleted", clear: deleteAction.clearError });

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
    shownReport,
    /** Show a saved report in the digest card. */
    pickReport: (id: number) => setSelected(id),
    /** Collect a saved report's period again and rebuild it (it replaces that report). */
    rerun: (periodLabel: string) => start({ kind: "collect", period: periodLabel }),
    /** Save a report as a PDF where the user picks ("Saved PDF"; nothing when they cancel). */
    exportPdf: (id: number, periodLabel: string) => void exportAction.run(id, periodLabel),
    /** Save the report on screen as a PDF; `undefined` while none is shown. */
    exportShown:
      shownReport !== undefined && shownPeriod !== undefined
        ? () => void exportAction.run(shownReport, shownPeriod)
        : undefined,
    exporting: exportAction.busy,
    /** Delete a saved report; resolves `true` once it is gone. */
    remove: async (id: number) => (await deleteAction.run(id)) === true,
    deleting: deleteAction.busy,
    adds,
  };
}
