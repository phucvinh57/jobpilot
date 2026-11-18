// The Trends screen's use-cases: read what it shows, start a run (new research, rewrite, continue,
// retry, rerun a saved report), pick the period, open a saved report and go back to the list,
// export or delete one, add a topic to the preferences.

import { useCallback, useRef, useState } from "react";

import { useErrorToast } from "@/hooks/useErrorToast";
import { useGraphProgress } from "@/hooks/useGraphProgress";
import { useAction, useQuery } from "@/hooks/useAsync";
import { toast } from "@/stores/toast";

import * as api from "@/api/trends";
import { reportFileName } from "@/lib/trends/format";
import type { Job } from "@/lib/trends/jobs";
import type { Cadence, Overview } from "@/types/trends";
import { useRuns } from "./runs";
import { usePreferenceAdds } from "./usePreferenceAdds";

/** What was read, and for which cadence and report (until the answer for a new one lands, its
 *  period or digest is not known). */
interface Loaded extends Overview {
  cadence: Cadence;
  selected: number | undefined;
}

/** Why a topic was not added to the preferences. */
function say(text: string) {
  toast.info(text);
}

export function useTrends() {
  const [cadence, setCadence] = useState<Cadence>("weekly");
  // The report that is open; `undefined`: the list.
  const [selected, setSelected] = useState<number>();

  const overview = useQuery<Loaded>(
    () => api.overview(selected, cadence).then((o) => ({ ...o, cadence, selected })),
    [selected, cadence],
    // Sources and digests of this screen; the preferences decide which topics read "Added".
    ["trends", "profile"],
  );
  const data = overview.data;

  const running = useRuns((s) => s.running);
  const step = useRuns((s) => s.step);
  const failed = useRuns((s) => s.notice);
  const clearFailure = useRuns((s) => s.clearFailure);
  useGraphProgress(useRuns.getState().follow);

  // The open report, once its digest has been read (until then `data` still holds the list's).
  const opened = selected !== undefined && data?.selected === selected;
  const shownId = opened ? data.digest?.reportId : selected;
  const shownRow = shownId === undefined ? undefined : data?.history.find((r) => r.id === shownId);

  // Where the user is when a run ends (the run outlives a render).
  const place = useRef({ selected, period: shownRow?.period });
  place.current = { selected, period: shownRow?.period };

  const start = useCallback(async (job: Job) => {
    const end = await useRuns.getState().start(job);
    // The new report opens for a user who waited on the list, or who looks at the one it replaces.
    // (After leaving the screen this instance is gone and nothing opens: the toast told them.)
    const now = place.current;
    if (end?.reportId != null && (now.selected === undefined || now.period === job.period)) {
      setSelected(end.reportId);
    }
  }, []);

  // A run that did not finish is a toast with Retry (carrying on where it stopped), shown once:
  // then the store forgets it. One that failed while the screen was closed toasts on return.
  useErrorToast(failed?.error, {
    title: "The run did not finish",
    onRetry: failed && (() => void start(failed.retry)),
    clear: clearFailure,
  });

  const adds = usePreferenceAdds(data, say);

  const exportAction = useAction(async (id: number, period: string) => {
    const fileName = reportFileName(period);
    const written = await api.exportPdf(id, fileName);
    if (written !== null) {
      // The name the user chose in the dialog, not the suggested one.
      toast.ok("Saved PDF", written.split(/[\\/]/).pop() || written);
    }
  });
  useErrorToast(exportAction.error, { title: "The report could not be saved as PDF", clear: exportAction.clearError });

  const deleteAction = useAction(async (id: number) => {
    await api.deleteReport(id);
    if (id === selected) {
      setSelected(undefined);
    }
    toast.ok("Report deleted");
    return true;
  });
  useErrorToast(deleteAction.error, { title: "The report could not be deleted", clear: deleteAction.clearError });

  // The period of the chosen cadence, once it has been read (a new research covers it).
  const period = data?.cadence === cadence ? data.period : undefined;
  const busy = running !== undefined;

  return {
    data,
    loading: overview.loading,
    /** The saved data could not be read. */
    loadError: overview.error,
    reload: overview.reload,
    /** The period of a new research, and the filter of the reports list. */
    cadence,
    chooseCadence: setCadence,
    busy,
    /** The run in flight: the period it builds and the step it is at. */
    running: running && { period: running.period, step: step ?? "" },
    /** The period label a new research would cover (`2026-W46`); `undefined` until it has been read. */
    periodLabel: period?.label,
    /** Start a new research for the chosen period: `true` when it started (it runs on its own, and
     *  says how it ended in a toast). */
    collect: (): boolean => {
      if (period === undefined || busy) {
        return false;
      }
      void start({ kind: "collect", period: period.label });
      toast.ok("Research started", `${period.title} (${period.label})`);
      return true;
    },
    /** "Continue" a collection paused at a login, captcha or block page. */
    resume: (periodLabel: string) => start({ kind: "resume", period: periodLabel }),
    /** The digest alone, from what is stored ("Build digest without it", "Rewrite digest"). */
    regenerateFor: (periodLabel: string) => start({ kind: "regenerate", period: periodLabel }),
    /** Collect a saved report's period again and rebuild it (it replaces that report). */
    rerun: (periodLabel: string) => start({ kind: "collect", period: periodLabel }),
    /** The open report; `undefined` on the list. */
    report:
      selected === undefined
        ? undefined
        : {
            /** Its digest has not been read yet. */
            loading: !opened,
            digest: opened ? data.digest : null,
            chart: opened ? data.chart : null,
            /** Its row in the list: the period its actions run for. */
            row: shownRow,
          },
    /** Open a saved report. */
    pickReport: (id: number) => setSelected(id),
    /** Back to the list. */
    back: () => setSelected(undefined),
    /** Save a report as a PDF where the user picks ("Saved PDF"; nothing when they cancel). */
    exportPdf: (id: number, periodLabel: string) => void exportAction.run(id, periodLabel),
    exporting: exportAction.busy,
    /** Delete a saved report; resolves `true` once it is gone. */
    remove: async (id: number) => (await deleteAction.run(id)) === true,
    deleting: deleteAction.busy,
    adds,
  };
}
