// The sources of the digest: add (Hacker News and YC at once, a feed or page from the form),
// enable or disable for the next run, remove.

import { useState } from "react";

import { useAction } from "@/shared";

import * as api from "../api";
import type { SourceKind, SourceKindInfo } from "../domain/types";

/** Run `work` as the one change in flight; `true` when it succeeded. */
async function done(work: () => Promise<void>): Promise<boolean> {
  await work();
  return true;
}

export function useSourceEditing() {
  /** The kind whose address form is open. */
  const [adding, setAdding] = useState<SourceKind>();
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const change = useAction(done);

  const closeForm = () => {
    setAdding(undefined);
    change.clearError();
  };

  return {
    adding,
    name,
    url,
    setName,
    setUrl,
    /** Why the last change failed (cleared by the next one). */
    error: change.error?.message,
    busy: change.busy,
    /** A kind that needs an address opens (or closes) its form; the others are added at once. */
    choose: (kind: SourceKindInfo) => {
      if (kind.needsUrl) {
        change.clearError();
        setAdding((open) => (open === kind.kind ? undefined : kind.kind));
        return Promise.resolve();
      }
      return change.run(() => api.addSource(kind.kind, "", "")).then(() => undefined);
    },
    /** Add the kind of the open form with the typed name and address, then close and clear it. */
    submit: async (kind: SourceKindInfo) => {
      const added = await change.run(() => api.addSource(kind.kind, name, url));
      if (added) {
        setAdding(undefined);
        setName("");
        setUrl("");
      }
    },
    cancel: closeForm,
    remove: (id: number) => change.run(() => api.removeSource(id)).then(() => undefined),
    setEnabled: (id: number, enabled: boolean) =>
      change.run(() => api.setSourceEnabled(id, enabled)).then(() => undefined),
  };
}
