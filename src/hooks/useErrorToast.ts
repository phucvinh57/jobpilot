// A failure held in a hook's state (`useAction().error`, `Runner.error`, `Query.error`) shown as a
// toast: once per error object, also when a view that still holds it mounts again; then `clear`
// forgets it, so the same failure can be reported again by the next attempt.

import { useEffect, useRef } from "react";

import type { CommandError } from "@/lib/problem";
import { toast, type ErrorToastOptions } from "@/stores/toast";

/** The errors already shown. */
const reported = new WeakSet<CommandError>();

export function useErrorToast(
  error: CommandError | undefined,
  options: ErrorToastOptions & { clear?: () => void } = {},
): void {
  const latest = useRef(options);
  latest.current = options;
  useEffect(() => {
    if (!error || reported.has(error)) {
      return;
    }
    reported.add(error);
    const { clear, ...rest } = latest.current;
    toast.error(error, rest);
    clear?.();
  }, [error]);
}
