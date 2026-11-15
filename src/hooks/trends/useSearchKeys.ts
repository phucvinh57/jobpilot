// The search API keys that switch report search on. Write-only: a typed key goes to the OS keyring
// through the shell and the field is cleared; the screen only ever knows whether one is saved.

import { useState } from "react";

import { useAction } from "@/hooks/useAsync";

import * as api from "@/api/trends";
import type { SearchKey, SearchProvider } from "@/types/trends";

async function done(work: () => Promise<void>): Promise<boolean> {
  await work();
  return true;
}

export function useSearchKeys() {
  const [typed, setTyped] = useState<Partial<Record<SearchProvider, string>>>({});
  const [shown, setShown] = useState<Partial<Record<SearchProvider, boolean>>>({});
  const [missing, setMissing] = useState<string>();
  const change = useAction(done);

  const forget = (provider: SearchProvider) => setTyped((before) => ({ ...before, [provider]: "" }));

  return {
    valueOf: (provider: SearchProvider) => typed[provider] ?? "",
    type: (provider: SearchProvider, value: string) => {
      setMissing(undefined);
      setTyped((before) => ({ ...before, [provider]: value }));
    },
    isShown: (provider: SearchProvider) => shown[provider] === true,
    toggleShown: (provider: SearchProvider) =>
      setShown((before) => ({ ...before, [provider]: before[provider] !== true })),
    /** Why the last change failed (cleared by the next one). */
    error: missing ?? change.error?.message,
    busy: change.busy,
    /** Save the typed key; the field is cleared afterwards. */
    save: async (key: SearchKey) => {
      const value = (typed[key.provider] ?? "").trim();
      if (value === "") {
        change.clearError();
        setMissing(`Enter the ${key.label} API key first.`);
        return;
      }
      setMissing(undefined);
      const saved = await change.run(() => api.saveKey(key.provider, value));
      if (saved) {
        forget(key.provider);
      }
    },
    clear: async (key: SearchKey) => {
      setMissing(undefined);
      await change.run(() => api.clearKey(key.provider));
    },
  };
}
