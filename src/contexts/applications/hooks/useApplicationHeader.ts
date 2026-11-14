import { useCallback, useEffect, useState } from "react";

import { useAction, useGraphProgress, useQuery, type CommandError } from "@/shared";

import { approveCv, dismissError, generate, onProgress, readApplication, retry, revise } from "../api/applicationsApi";
import type { ApplicationHeader, JobProgress, Primary } from "../domain/types";
import { recheckOnFailure } from "./recheck";

export interface HeaderModel {
  /** `undefined` until the first read; `null` when the application does not exist (any more). */
  header: ApplicationHeader | null | undefined;
  loadError: CommandError | undefined;
  /** The progress line of the running generate / revise job, moving with every step. */
  jobLabel: string | null;
  /** The primary action; the Apply tab's answer wins until the header is read again. */
  primary: Primary | null;
  /** The Apply tab learned the primary action changed (what was typed, a review opened). */
  setPrimary: (primary: Primary | null) => void;
  /** The last failure of a job: the shell keeps it until the next job starts or it is dismissed. */
  error: CommandError | null;
  /** Bumps whenever something rewrote the CV files: the CV tab reads them again. */
  cvEpoch: number;
  generate: () => void;
  approve: () => void;
  retry: () => void;
  dismiss: () => void;
  /** "Ask AI": resolves when the job ended; the failure is read back from the header. */
  revise: (instruction: string, yaml: string) => Promise<void>;
}

/**
 * The header of one application and the jobs that change it. The shell owns the job state (one job per
 * application, its progress, its last failure), so a job survives leaving the screen; this hook only
 * reads it and starts jobs.
 */
export function useApplicationHeader(id: number): HeaderModel {
  const query = useQuery(() => readApplication(id), [id], ["applications"]);
  const header = query.data;

  // Progress lines come faster than the header is read; the header's own line is the fallback.
  const [live, setLive] = useState<JobProgress | null>(null);
  useEffect(
    () =>
      onProgress((p) => {
        if (p.appId === id) {
          setLive(p);
        }
      }),
    [id],
  );
  if (header && header.job === null && live !== null) {
    setLive(null);
  }
  const jobLabel = header?.job == null ? null : live !== null ? live.label : header.job;

  // A CV made by the "Prepare CV" graph (from the Leads) changes the files without a job of this screen.
  const [graphRuns, setGraphRuns] = useState(0);
  const reload = query.reload;
  useGraphProgress((p) => {
    if (p.graph === "prepare_cv" && (p.kind === "done" || p.kind === "failed" || p.kind === "cancelled")) {
      setGraphRuns((n) => n + 1);
      reload();
    }
  });

  const [override, setOverride] = useState<{ header: ApplicationHeader; primary: Primary | null }>();
  const primary = header == null ? null : override?.header === header ? override.primary : header.primary;
  const setPrimary = useCallback(
    (next: Primary | null) => {
      if (header) {
        setOverride({ header, primary: next });
      }
    },
    [header],
  );

  const generateAction = useAction(() => recheckOnFailure(() => generate(id)));
  const approveAction = useAction(() => recheckOnFailure(() => approveCv(id)));
  const retryAction = useAction(() => recheckOnFailure(() => retry(id)));
  const dismissAction = useAction(() => dismissError(id));
  const reviseAction = useAction((instruction: string, yaml: string) => recheckOnFailure(() => revise(id, instruction, yaml)));

  return {
    header,
    loadError: query.error,
    jobLabel,
    primary,
    setPrimary,
    error: header?.error ?? null,
    cvEpoch: (header?.cvEpoch ?? 0) + graphRuns,
    generate: () => void generateAction.run(),
    approve: () => void approveAction.run(),
    retry: () => void retryAction.run(),
    dismiss: () => void dismissAction.run(),
    revise: async (instruction, yaml) => {
      await reviseAction.run(instruction, yaml);
    },
  };
}
