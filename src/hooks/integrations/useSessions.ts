// The use-cases of Settings > Browser: the connection to the JobPilot extension (pairing, forget) and
// the sites the user is signed in to. The section re-reads itself whenever the connection changes
// (`integrations:browser-status`). Several sites can run something at once (each row has its own
// busy label); Delete asks "Sure?" first. A new site is named in a dialog that closes once its
// sign-in tab is open; a failure to add it stays in the dialog, every other failure is a toast.

import { useCallback, useEffect, useRef, useState } from "react";

import { CommandError, toCommandError } from "@/lib/problem";
import { useQuery, type Query } from "@/hooks/useAsync";

import * as api from "@/api/sessions";
import { FIRST_SUGGESTION, NEW_KEY } from "@/lib/integrations/sessions";
import { toast } from "@/stores/toast";
import type { BrowserSection } from "@/types/integrations";

/** What runs on the browser connection (pairing, forgetting). */
export type ConnectionWork = "pair" | "decline" | "forget";

export interface SessionsCard {
  query: Query<BrowserSection>;
  /** Running operations by site id (`NEW_KEY` for adding one) with the label to show. */
  busy: Readonly<Record<number, string>>;
  /** The site whose Delete asks "Sure?". */
  confirmDelete: number | undefined;
  /** The failure of an action (shown as a toast). */
  error: CommandError | undefined;
  /** What failed: the title of that toast. */
  errorTitle: string;
  clearError: () => void;
  /** The pairing request the dialog shows (`undefined`: none, or the user closed it). */
  pairingCode: string | undefined;
  /** Show the pairing dialog again after it was closed without an answer. */
  reviewPairing: () => void;
  closePairing: () => void;
  approvePairing: () => void;
  declinePairing: () => void;
  /** The "Forget this browser?" dialog is open. */
  forgetOpen: boolean;
  openForget: () => void;
  closeForget: () => void;
  forgetBrowser: () => void;
  /** Pairing or forgetting runs. */
  connectionBusy: ConnectionWork | undefined;
  copyPath: (path: string) => void;
  /** The "Add a site" dialog is open. */
  formOpen: boolean;
  /** Why the site of the dialog could not be added (shown in the dialog). */
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
  /** The first click arms the confirmation, the second deletes. */
  remove: (id: number) => void;
}

interface Failure {
  error: CommandError;
  title: string;
}

export function useSessions(): SessionsCard {
  const query = useQuery(api.loadBrowser, []);
  const { reload } = query;
  const [busy, setBusy] = useState<Record<number, string>>({});
  const [confirmDelete, setConfirmDelete] = useState<number>();
  const [failure, setFailure] = useState<Failure>();
  const [createError, setCreateError] = useState<CommandError>();
  const [formOpen, setFormOpen] = useState(false);
  const [forgetOpen, setForgetOpen] = useState(false);
  const [connectionBusy, setConnectionBusy] = useState<ConnectionWork>();
  /** The pairing code the user closed the dialog of without answering. */
  const [setAside, setSetAside] = useState<string>();
  /** The pairing code the user approved here, until that pairing ends (the extension's popup has to
   *  say Pair too). */
  const [approved, setApproved] = useState<string>();
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const alive = useRef(true);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  // The extension connects, asks to pair or goes away while the section is open.
  useEffect(() => api.onBrowserStatus(reload), [reload]);

  const clearError = useCallback(() => setFailure(undefined), []);
  const code = query.data?.browser.pairingCode ?? undefined;
  const pairingCode = code !== undefined && code !== setAside ? code : undefined;

  // The pairing the user approved ended: paired once the extension said Pair too.
  const browser = query.data?.browser;
  useEffect(() => {
    if (approved === undefined || browser === undefined || browser.pairingCode === approved) {
      return;
    }
    setApproved(undefined);
    if (browser.connected) {
      toast.ok("Browser paired", "JobPilot can now open tabs in this browser.");
    }
  }, [approved, browser]);

  /** Run `call` for the site `key`; the section is read again whatever the outcome. A failure goes
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
      setConfirmDelete(undefined);
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

  /** Pair, decline or forget; `onOk` says how it went. A failure closes the dialog (`onFailed`) so
   *  its toast is seen, and the section is read again. */
  const connection = useCallback(
    (work: ConnectionWork, title: string, call: () => Promise<void>, onOk: () => void, onFailed: () => void) => {
      if (connectionBusy !== undefined) {
        return;
      }
      setConnectionBusy(work);
      call()
        .then(
          () => {
            if (alive.current) {
              onOk();
            }
          },
          (thrown: unknown) => {
            if (alive.current) {
              onFailed();
              setFailure({ error: toCommandError(thrown), title });
            }
          },
        )
        .finally(() => {
          if (alive.current) {
            setConnectionBusy(undefined);
          }
          reload();
        });
    },
    [connectionBusy, reload],
  );

  const approvePairing = useCallback(
    () =>
      connection(
        "pair",
        "Could not pair the browser",
        api.approvePairing,
        // Paired once the extension's popup says Pair too (at once if it already did).
        () => setApproved(code),
        () => setSetAside(code),
      ),
    [connection, code],
  );
  const declinePairing = useCallback(
    () =>
      connection(
        "decline",
        "Could not decline the browser",
        api.declinePairing,
        () => toast.info("Pairing declined", "That browser was not paired."),
        () => setSetAside(code),
      ),
    [connection, code],
  );
  const closePairing = useCallback(() => setSetAside(code), [code]);
  const reviewPairing = useCallback(() => setSetAside(undefined), []);

  const openForget = useCallback(() => setForgetOpen(true), []);
  const closeForget = useCallback(() => setForgetOpen(false), []);
  const forgetBrowser = useCallback(
    () =>
      connection(
        "forget",
        "Could not forget the browser",
        api.forgetBrowser,
        () => {
          setForgetOpen(false);
          toast.ok("Browser forgotten", "Pair it again to let JobPilot open tabs there.");
        },
        () => setForgetOpen(false),
      ),
    [connection],
  );

  const copyPath = useCallback((path: string) => {
    const copied = navigator.clipboard?.writeText(path);
    if (!copied) {
      setFailure({ error: new CommandError("The clipboard is not available."), title: "Could not copy the folder" });
      return;
    }
    copied.then(
      () => toast.ok("Folder copied", path),
      (thrown: unknown) => {
        if (alive.current) {
          setFailure({ error: toCommandError(thrown), title: "Could not copy the folder" });
        }
      },
    );
  }, []);

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
      "Opening the sign-in tab...",
      "Could not add the site",
      () => api.createSession(name, url),
      // The sign-in tab is open: the new row says to sign in there, then press Done.
      () => {
        setFormOpen(false);
        toast.info("Sign in in your browser", "Then press Done on the site's row.");
      },
      setCreateError,
    );
  }, [run, busy, name, url]);

  const openLogin = useCallback(
    (id: number) =>
      void run(id, "Opening the sign-in tab...", "Could not open the sign-in tab", () => api.openSession(id)),
    [run],
  );
  const finishLogin = useCallback(
    (id: number) => void run(id, "Checking...", "Could not finish signing in", () => api.finishLogin(id)),
    [run],
  );
  const check = useCallback(
    (id: number) => void run(id, "Checking...", "The check failed", () => api.checkSession(id)),
    [run],
  );

  const remove = useCallback(
    (id: number) => {
      if (confirmDelete !== id) {
        setConfirmDelete(id);
        return;
      }
      void run(id, "Deleting...", "Could not delete the site", () => api.deleteSession(id));
    },
    [confirmDelete, run],
  );

  return {
    query,
    busy,
    confirmDelete,
    error: failure?.error,
    errorTitle: failure?.title ?? "Something went wrong",
    clearError,
    pairingCode,
    reviewPairing,
    closePairing,
    approvePairing,
    declinePairing,
    forgetOpen,
    openForget,
    closeForget,
    forgetBrowser,
    connectionBusy,
    copyPath,
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
    remove,
  };
}
