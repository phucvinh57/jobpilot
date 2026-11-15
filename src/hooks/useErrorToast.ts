// A failure held in a hook's state (`useAction().error`, `Runner.error`, `Query.error`) shown as a
// toast: once per new error, then `clear` forgets it, so the same failure can be reported again.

import { useEffect, useRef } from "react";

import type { CommandError } from "@/lib/problem";
import { toast, type ErrorToastOptions } from "@/stores/toast";

export function useErrorToast(
  error: CommandError | undefined,
  options: ErrorToastOptions & { clear?: () => void } = {},
): void {
  const latest = useRef(options);
  latest.current = options;
  useEffect(() => {
    if (!error) {
      return;
    }
    const { clear, ...rest } = latest.current;
    toast.error(error, rest);
    clear?.();
  }, [error]);
}
