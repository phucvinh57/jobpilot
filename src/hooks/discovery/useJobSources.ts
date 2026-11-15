// Settings > Job sources as a screen needs it: the sources with their last run, the sessions they
// can be read in, and the changes the user makes (each one reads the list again through the
// `sources` notification the api layer sends).

import { useCallback, useState } from "react";

import { toCommandError, type CommandError } from "@/lib/problem";
import { useAction, useQuery } from "@/hooks/useAsync";
import { useGraphProgress } from "@/hooks/useGraphProgress";

import * as api from "@/api/discovery";
import { movesTheQueue } from "@/lib/discovery/progress";
import { rowName } from "@/lib/discovery/sources";
import { toast } from "@/stores/toast";
import type { JobSources } from "@/types/discovery";

export interface JobSourcesView {
  data: JobSources | undefined;
  /** The list could not be read. */
  loadError: CommandError | undefined;
  /** The first read has answered (before that the list is not "empty", it is not there yet). */
  loaded: boolean;
  /** Read the list again (a failed read has a Retry). */
  reload: () => void;
  /** Why the last change of a listed source (or of the daily switch) failed. */
  changeError: CommandError | undefined;
  clearChangeError: () => void;
  /** "Search daily while JobPilot is open", shown at once and until the list is read again. */
  daily: boolean;
  setDaily: (on: boolean) => void;
  setEnabled: (id: number, enabled: boolean) => void;
  setSession: (id: number, sessionId: number | null) => void;
  remove: (id: number) => void;
  /** Add a source; resolves to `true` when it was added (the dialog can close: a toast says so). */
  add: (kind: string, name: string, url: string) => Promise<boolean>;
  adding: boolean;
  /** Why the add dialog's last submit failed (shown in the dialog). */
  addError: CommandError | undefined;
  clearAddError: () => void;
}

export function useJobSources(): JobSourcesView {
  const query = useQuery(api.readJobSources, [], ["sources"]);
  const [changeError, setChangeError] = useState<CommandError>();
  const [pendingDaily, setPendingDaily] = useState<{ on: boolean; readBefore: JobSources | undefined }>();
  const addSource = useAction(api.addSource);

  // A source that ended, paused or failed has a new status.
  const { reload, data } = query;
  useGraphProgress((p) => {
    if (movesTheQueue(p)) {
      reload();
    }
  });

  /** Run a change; the others are not held up by it. */
  const change = useCallback(async (work: () => Promise<unknown>): Promise<boolean> => {
    setChangeError(undefined);
    try {
      await work();
      return true;
    } catch (thrown) {
      setChangeError(toCommandError(thrown));
      return false;
    }
  }, []);

  const setDaily = useCallback(
    (on: boolean) => {
      setPendingDaily({ on, readBefore: data });
      // A failed write puts the switch back; a saved one stays shown until the list is read again.
      void change(() => api.setDaily(on)).then((saved) => {
        if (!saved) {
          setPendingDaily(undefined);
        }
      });
    },
    [change, data],
  );

  const { run: runAdd, busy: adding, error: addError, clearError: clearAddError } = addSource;
  const clearChangeError = useCallback(() => setChangeError(undefined), []);
  const kinds = data?.kinds;
  const add = useCallback(
    async (kind: string, name: string, url: string): Promise<boolean> => {
      const added = await runAdd(kind, name, url);
      if (added === undefined) {
        return false;
      }
      toast.ok("Source added", rowName(added, kinds ?? []));
      return true;
    },
    [runAdd, kinds],
  );

  return {
    data,
    loadError: query.error,
    loaded: data !== undefined,
    reload,
    changeError,
    clearChangeError,
    daily: pendingDaily !== undefined && pendingDaily.readBefore === data ? pendingDaily.on : (data?.daily ?? false),
    setDaily,
    setEnabled: (id, enabled) => void change(() => api.setSourceEnabled(id, enabled)),
    setSession: (id, sessionId) => void change(() => api.setSourceSession(id, sessionId)),
    remove: (id) => void change(() => api.removeSource(id)),
    add,
    adding,
    addError,
    clearAddError,
  };
}
