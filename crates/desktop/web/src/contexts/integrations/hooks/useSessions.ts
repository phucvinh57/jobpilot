// The use-cases of Settings > Browser sessions. Several sessions can run something at once (each row
// has its own busy label); Sign out and Delete ask "Sure?" first; the browser download reports its
// percentage through events, also when the card was opened after it started.

import { useCallback, useEffect, useRef, useState } from "react";

import { toCommandError, useQuery, type CommandError, type Query } from "@/shared";

import * as api from "../api/sessionsApi";
import { FIRST_SUGGESTION, NEW_KEY, type Destructive, type Download, type Sessions } from "../domain/sessions";

export interface SessionsCard {
  query: Query<Sessions>;
  /** Running operations by session id (`NEW_KEY` for creation) with the label to show. */
  busy: Readonly<Record<number, string>>;
  confirm: { id: number; action: Destructive } | undefined;
  error: CommandError | undefined;
  clearError: () => void;
  download: Download;
  formOpen: boolean;
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

export function useSessions(): SessionsCard {
  const query = useQuery(api.loadSessions, []);
  const { reload } = query;
  const [busy, setBusy] = useState<Record<number, string>>({});
  const [confirm, setConfirm] = useState<{ id: number; action: Destructive }>();
  const [error, setError] = useState<CommandError>();
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

  /** Run `call` for the session `key`; the list is read again whatever the outcome. */
  const run = useCallback(
    async <T>(key: number, label: string, call: () => Promise<T>, onOk?: (value: T) => void) => {
      setBusy((b) => ({ ...b, [key]: label }));
      setConfirm(undefined);
      setError(undefined);
      try {
        const value = await call();
        if (alive.current) {
          onOk?.(value);
        }
      } catch (thrown) {
        if (alive.current) {
          setError(toCommandError(thrown));
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
    setFormOpen(true);
    fill(FIRST_SUGGESTION.name, FIRST_SUGGESTION.url);
  }, [fill]);

  const createAndLogin = useCallback(() => {
    void run(NEW_KEY, "Opening browser...", () => api.createSession(name, url), () => setFormOpen(false));
  }, [run, name, url]);

  const openLogin = useCallback((id: number) => void run(id, "Opening browser...", () => api.openSession(id)), [run]);
  const finishLogin = useCallback((id: number) => void run(id, "Checking...", () => api.finishLogin(id)), [run]);
  const check = useCallback((id: number) => void run(id, "Checking...", () => api.checkSession(id)), [run]);

  const destructive = useCallback(
    (id: number, action: Destructive) => {
      if (confirm?.id !== id || confirm.action !== action) {
        setConfirm({ id, action });
        return;
      }
      if (action === "signOut") {
        void run(id, "Signing out...", () => api.signOutSession(id));
      } else {
        void run(id, "Deleting...", () => api.deleteSession(id));
      }
    },
    [confirm, run],
  );

  const downloadBrowser = useCallback(() => {
    if (download.active) {
      return;
    }
    setError(undefined);
    setLive({ active: true, percent: null });
    api.downloadBrowser().then(
      () => undefined,
      (thrown: unknown) => {
        if (alive.current) {
          setError(toCommandError(thrown));
        }
      },
    ).finally(() => {
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
    error,
    clearError: () => setError(undefined),
    download,
    formOpen,
    name,
    url,
    setName,
    setUrl,
    openForm,
    closeForm: () => setFormOpen(false),
    fill,
    createAndLogin,
    openLogin,
    finishLogin,
    check,
    destructive,
    downloadBrowser,
  };
}
