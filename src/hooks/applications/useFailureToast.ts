// Failures of this feature reported as toasts. A failure the shell keeps in what it returns (the last
// job's, the apply run's) comes back as a new object with every read: it is reported once, not again
// with each read that still carries it.

import { useEffect } from "react";

import { useErrorToast } from "@/hooks/useErrorToast";
import { sameFailure } from "@/lib/applications/failure";
import type { CommandError } from "@/lib/problem";
import type { ErrorToastOptions } from "@/stores/toast";

import type { HeaderModel } from "./useApplicationHeader";

/** The failure last seen per place (`key`), kept across mounts: a failure the shell still holds is
 *  not reported again when its screen opens again. */
const seen = new Map<string, CommandError>();

/** `error`, as the object it had when this failure was first seen at `key`. */
export function useSameFailure(key: string, error: CommandError | null | undefined): CommandError | undefined {
  const before = seen.get(key);
  const kept = sameFailure(before, error) ? before : (error ?? undefined);
  useEffect(() => {
    if (kept) {
      seen.set(key, kept);
    } else {
      seen.delete(key);
    }
  }, [key, kept]);
  return kept;
}

/** A failure kept by the shell, as one toast per failure. */
export function useKeptFailureToast(
  key: string,
  error: CommandError | null | undefined,
  options: ErrorToastOptions & { clear?: () => void },
): void {
  useErrorToast(useSameFailure(key, error), options);
}

/** The failure of the last job (Generate, Ask AI, Approve CV), with Retry. It stays until the
 *  toast is closed, then it is dismissed in the shell (the timeline keeps it as an event). */
export function useJobFailureToast(id: number, head: HeaderModel): void {
  useKeptFailureToast(`job:${id}`, head.error, {
    title: "The last job failed",
    onRetry: head.retry,
    onDismiss: head.dismiss,
  });
}
