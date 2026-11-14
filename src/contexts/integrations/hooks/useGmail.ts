// The use-cases of Settings > Gmail: save the OAuth client and sign in through the system browser
// (cancellable), sync now, disconnect (two steps). The client secret is typed here, goes to the
// shell and the OS keyring, and is never read back; the card only learns whether one is saved.

import { useCallback, useEffect, useState } from "react";

import { useQuery, type Query } from "@/shared";

import * as api from "../api/gmailApi";
import type { Gmail } from "../domain/gmail";
import { useRunner, type Runner } from "./useRunner";

export interface GmailCard {
  query: Query<Gmail>;
  runner: Runner;
  /** The client id box: what was typed, else the saved one. */
  clientId: string;
  setClientId: (text: string) => void;
  clientSecret: string;
  setClientSecret: (text: string) => void;
  connecting: boolean;
  confirmDisconnect: boolean;
  connect: () => void;
  cancelConnect: () => void;
  syncNow: () => void;
  /** First click arms the button, the second disconnects. */
  disconnect: () => void;
}

export function useGmail(): GmailCard {
  const query = useQuery(api.loadGmail, []);
  const runner = useRunner();
  const [typedId, setTypedId] = useState<string>();
  const [clientSecret, setClientSecret] = useState("");
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const { reload } = query;
  const { fail, run, setNotice } = runner;

  // A sync in the background (the poller) stored mail: the "last sync" line is stale.
  useEffect(() => api.onGmailChanged(reload), [reload]);

  const clientId = typedId ?? query.data?.clientId ?? "";

  const connect = useCallback(() => {
    const id = clientId.trim();
    if (id === "") {
      fail("Enter the OAuth client ID.", "Could not connect");
      return;
    }
    const secret = clientSecret.trim();
    void run("connecting", "Could not connect", () => api.connect(id, secret === "" ? null : secret)).then((account) => {
      if (account !== undefined) {
        // The box is emptied: what was typed is in the keyring now.
        setClientSecret("");
        reload();
        setNotice(`Connected as ${account}. Syncing…`);
      }
    });
  }, [clientId, clientSecret, fail, run, reload, setNotice]);

  const cancelConnect = useCallback(() => {
    api.cancelConnect().catch(() => undefined);
  }, []);

  const syncNow = useCallback(() => {
    setConfirmDisconnect(false);
    // The outcome (or the error) is saved by the sync and shown by the card.
    void run("syncing", "Sync failed", () => api.syncNow()).then(() => reload());
  }, [run, reload]);

  const disconnect = useCallback(() => {
    if (!confirmDisconnect) {
      setConfirmDisconnect(true);
      return;
    }
    setConfirmDisconnect(false);
    void run("disconnecting", "Could not disconnect", () => api.disconnect().then(() => true)).then((ok) => {
      reload();
      if (ok) {
        setNotice("Disconnected.");
      }
    });
  }, [confirmDisconnect, run, reload, setNotice]);

  return {
    query,
    runner,
    clientId,
    setClientId: setTypedId,
    clientSecret,
    setClientSecret,
    connecting: runner.busy === "connecting",
    confirmDisconnect,
    connect,
    cancelConnect,
    syncNow,
    disconnect,
  };
}
