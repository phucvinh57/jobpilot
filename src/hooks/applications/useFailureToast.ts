// Failures of this feature reported as toasts. A failure the shell keeps in what it returns (the last
// job's, the apply run's) comes back as a new object with every read: it is reported once, not again
// with each read that still carries it.

import { useRef } from "react";

import { useErrorToast } from "@/hooks/useErrorToast";
import { sameFailure } from "@/lib/applications/failure";
import type { CommandError } from "@/lib/problem";
import type { ErrorToastOptions } from "@/stores/toast";

import type { HeaderModel } from "./useApplicationHeader";

/** `error`, as the object it had when this failure was first seen. */
export function useSameFailure(error: CommandError | null | undefined): CommandError | undefined {
  const seen = useRef<CommandError | undefined>(undefined);
  if (!sameFailure(seen.current, error)) {
    seen.current = error ?? undefined;
  }
  return seen.current;
}

/** A failure kept by the shell, as one toast per failure. */
export function useKeptFailureToast(
  error: CommandError | null | undefined,
  options: ErrorToastOptions & { clear?: () => void },
): void {
  useErrorToast(useSameFailure(error), options);
}

/** The failure of the last job (Generate, Ask AI, Approve CV), with Retry. Shown once: it is then
 *  dismissed in the shell (the timeline keeps it as an event). */
export function useJobFailureToast(head: HeaderModel): void {
  useKeptFailureToast(head.error, { title: "The last job failed", onRetry: head.retry, clear: head.dismiss });
}
