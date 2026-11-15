// The sources of the digest: add (Hacker News and YC at once, a feed or page from a dialog),
// enable or disable for the next run, remove. A failed add from the dialog stays in the dialog;
// a failed change of the list (an add at once, the switch, Remove) is reported as a toast.

import { useCallback, useState } from "react";

import { useAction } from "@/hooks/useAsync";

import * as api from "@/api/trends";
import { toast } from "@/stores/toast";
import type { SourceKind, SourceKindInfo } from "@/types/trends";

/** Run `work` as the one change in flight; `true` when it succeeded. */
async function done(work: () => Promise<void>): Promise<boolean> {
  await work();
  return true;
}

export function useSourceEditing() {
  /** The kind whose address dialog is open. */
  const [adding, setAdding] = useState<SourceKind>();
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  /** What the last change of the list was, for the title of its failure. */
  const [what, setWhat] = useState("Could not change the source");
  const change = useAction(done);
  const add = useAction(done);
  const { clearError: clearAddError } = add;

  const listChange = (title: string, work: () => Promise<void>) => {
    setWhat(title);
    return change.run(work).then(() => undefined);
  };

  const close = useCallback(() => {
    setAdding(undefined);
    clearAddError();
  }, [clearAddError]);

  return {
    adding,
    name,
    url,
    setName,
    setUrl,
    /** Why the add from the dialog failed (shown in the dialog; cleared by the next try). */
    formError: add.error?.message,
    /** A failed change of the list, and its title (shown as a toast, then cleared). */
    listError: change.error,
    listErrorTitle: what,
    clearListError: change.clearError,
    busy: change.busy || add.busy,
    /** A kind that needs an address opens its dialog; the others are added at once. */
    choose: (kind: SourceKindInfo) => {
      if (kind.needsUrl) {
        clearAddError();
        setAdding(kind.kind);
        return Promise.resolve();
      }
      return listChange(`Could not add ${kind.label}`, () => api.addSource(kind.kind, "", ""));
    },
    /** Add the kind of the open dialog with the typed name and address, then close and clear it. */
    submit: async (kind: SourceKindInfo) => {
      const typedName = name.trim();
      const typedUrl = url.trim();
      const added = await add.run(() => api.addSource(kind.kind, name, url));
      if (added) {
        setAdding(undefined);
        setName("");
        setUrl("");
        toast.ok("Source added", typedName || typedUrl || kind.label);
      }
    },
    cancel: close,
    remove: (id: number) => listChange("Could not remove the source", () => api.removeSource(id)),
    setEnabled: (id: number, enabled: boolean) =>
      listChange(enabled ? "Could not turn the source on" : "Could not turn the source off", () =>
        api.setSourceEnabled(id, enabled),
      ),
  };
}
