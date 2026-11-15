// The use-cases of Settings > Gmail: save the OAuth client and sign in through the system browser
// (cancellable), sync now, disconnect (two steps). The client secret is typed here, goes to the
// shell and the OS keyring, and is never read back; the card only learns whether one is saved.
// Connect belongs to the client form (its failure shows under the fields); Sync now and Disconnect
// are actions of the connected card (their failures are toasts).

import { useCallback, useEffect, useState } from "react";

import { useQuery, type Query } from "@/hooks/useAsync";

import * as api from "@/api/gmail";
import { toast } from "@/stores/toast";
import type { Gmail } from "@/types/integrations";
import { useRunner, type Runner } from "./useRunner";

export interface GmailCard {
  query: Query<Gmail>;
  /** Connect: the signed-out form. */
  connection: Runner;
  /** Sync now and Disconnect: the signed-in card. */
  actions: Runner;
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
  const connection = useRunner();
  const actions = useRunner();
  const [typedId, setTypedId] = useState<string>();
  const [clientSecret, setClientSecret] = useState("");
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const { reload } = query;
  const { fail, run: runConnect } = connection;
  const { run } = actions;

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
    void runConnect("connecting", "Could not connect", () => api.connect(id, secret === "" ? null : secret)).then(
      (account) => {
        if (account !== undefined) {
          // The box is emptied: what was typed is in the keyring now.
          setClientSecret("");
          reload();
          toast.ok("Gmail connected", `Connected as ${account}. Syncing…`);
        }
      },
    );
  }, [clientId, clientSecret, fail, runConnect, reload]);

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
        toast.ok("Gmail disconnected");
      }
    });
  }, [confirmDisconnect, run, reload]);

  return {
    query,
    connection,
    actions,
    clientId,
    setClientId: setTypedId,
    clientSecret,
    setClientSecret,
    connecting: connection.busy === "connecting",
    confirmDisconnect,
    connect,
    cancelConnect,
    syncNow,
    disconnect,
  };
}
