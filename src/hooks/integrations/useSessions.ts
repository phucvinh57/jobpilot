// The use-cases of Settings > Browser sessions. Several sessions can run something at once (each row
// has its own busy label); Sign out and Delete ask "Sure?" first; the browser download reports its
// percentage through events, also when the card was opened after it started. A new session is
// named in a dialog that closes once the login browser is open; a failure to create it stays in
// the dialog, the failure of a row action or of the download is reported as a toast.

import { useCallback, useEffect, useRef, useState } from "react";

import { toCommandError, type CommandError } from "@/lib/problem";
import { useQuery, type Query } from "@/hooks/useAsync";

import * as api from "@/api/sessions";
import { FIRST_SUGGESTION, NEW_KEY, type Destructive } from "@/lib/integrations/sessions";
import { toast } from "@/stores/toast";
import type { Download, Sessions } from "@/types/integrations";

export interface SessionsCard {
  query: Query<Sessions>;
  /** Running operations by session id (`NEW_KEY` for creation) with the label to show. */
  busy: Readonly<Record<number, string>>;
  confirm: { id: number; action: Destructive } | undefined;
  /** The failure of a row action or of the download (shown as a toast). */
  error: CommandError | undefined;
  /** What failed: the title of that toast. */
  errorTitle: string;
  clearError: () => void;
  download: Download;
  /** The "New session" dialog is open. */
  formOpen: boolean;
  /** Why the session of the dialog could not be created (shown in the dialog). */
  createError: CommandError | undefined;
  name: string;
  url: string;
  setName: (name: string) => void;
  setUrl: (url: string) => void;
  openForm: () => void;
  closeForm: () => void;
  fill: (name: string, url: string) => void;
  createAndLogin: () => void;
  openLogin: (id: number) => void;
  finishLogin: (id: number) => void;
  check: (id: number) => void;
  /** First click arms the confirmation, the second runs the action. */
  destructive: (id: number, action: Destructive) => void;
  downloadBrowser: () => void;
}

const NO_DOWNLOAD: Download = { active: false, percent: null };

interface Failure {
  error: CommandError;
  title: string;
}

export function useSessions(): SessionsCard {
  const query = useQuery(api.loadSessions, []);
  const { reload } = query;
  const [busy, setBusy] = useState<Record<number, string>>({});
  const [confirm, setConfirm] = useState<{ id: number; action: Destructive }>();
  const [failure, setFailure] = useState<Failure>();
  const [createError, setCreateError] = useState<CommandError>();
  const [live, setLive] = useState<Download>();
  const [formOpen, setFormOpen] = useState(false);
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The download's percentage moves while it runs; when it ends the browser line is read again.
  useEffect(
    () =>
      api.onBrowserDownload((download) => {
        setLive(download);
        if (!download.active) {
          reload();
        }
      }),
    [reload],
  );

  const download = live ?? query.data?.download ?? NO_DOWNLOAD;
  const clearError = useCallback(() => setFailure(undefined), []);

  /** Run `call` for the session `key`; the list is read again whatever the outcome. A failure goes
   *  to `onError`, else it is the card's failure titled `title`. */
  const run = useCallback(
    async <T>(
      key: number,
      label: string,
      title: string,
      call: () => Promise<T>,
      onOk?: (value: T) => void,
      onError?: (error: CommandError) => void,
    ) => {
      setBusy((b) => ({ ...b, [key]: label }));
      setConfirm(undefined);
      try {
        const value = await call();
        if (alive.current) {
          onOk?.(value);
        }
      } catch (thrown) {
        if (alive.current) {
          const error = toCommandError(thrown);
          if (onError) {
            onError(error);
          } else {
            setFailure({ error, title });
          }
        }
      } finally {
        if (alive.current) {
          setBusy((b) => {
            const rest = { ...b };
            delete rest[key];
            return rest;
          });
        }
        reload();
      }
    },
    [reload],
  );

  const fill = useCallback((suggestedName: string, suggestedUrl: string) => {
    setName(suggestedName);
    setUrl(suggestedUrl);
  }, []);

  const openForm = useCallback(() => {
    setCreateError(undefined);
    setFormOpen(true);
    fill(FIRST_SUGGESTION.name, FIRST_SUGGESTION.url);
  }, [fill]);

  const closeForm = useCallback(() => {
    setCreateError(undefined);
    setFormOpen(false);
  }, []);

  const createAndLogin = useCallback(() => {
    if (busy[NEW_KEY] !== undefined) {
      return;
    }
    setCreateError(undefined);
    void run(
      NEW_KEY,
      "Opening browser...",
      "Could not create the session",
      () => api.createSession(name, url),
      // The login browser is open: its row says to log in there, then press Done.
      () => {
        setFormOpen(false);
        toast.info("Log in in the browser window", "Then press Done on the session's row.");
      },
      setCreateError,
    );
  }, [run, busy, name, url]);

  const openLogin = useCallback(
    (id: number) => void run(id, "Opening browser...", "Could not open the browser", () => api.openSession(id)),
    [run],
  );
  const finishLogin = useCallback(
    (id: number) => void run(id, "Checking...", "Could not finish the login", () => api.finishLogin(id)),
    [run],
  );
  const check = useCallback(
    (id: number) => void run(id, "Checking...", "The check failed", () => api.checkSession(id)),
    [run],
  );

  const destructive = useCallback(
    (id: number, action: Destructive) => {
      if (confirm?.id !== id || confirm.action !== action) {
        setConfirm({ id, action });
        return;
      }
      if (action === "signOut") {
        void run(id, "Signing out...", "Could not sign out", () => api.signOutSession(id));
      } else {
        void run(id, "Deleting...", "Could not delete the session", () => api.deleteSession(id));
      }
    },
    [confirm, run],
  );

  const downloadBrowser = useCallback(() => {
    if (download.active) {
      return;
    }
    setLive({ active: true, percent: null });
    api
      .downloadBrowser()
      .then(
        () => undefined,
        (thrown: unknown) => {
          if (alive.current) {
            setFailure({ error: toCommandError(thrown), title: "Could not download the browser" });
          }
        },
      )
      .finally(() => {
        if (alive.current) {
          setLive(NO_DOWNLOAD);
          reload();
        }
      });
  }, [download.active, reload]);

  return {
    query,
    busy,
    confirm,
    error: failure?.error,
    errorTitle: failure?.title ?? "Something went wrong",
    clearError,
    download,
    formOpen,
    createError,
    name,
    url,
    setName,
    setUrl,
    openForm,
    closeForm,
    fill,
    createAndLogin,
    openLogin,
    finishLogin,
    check,
    destructive,
    downloadBrowser,
  };
}
