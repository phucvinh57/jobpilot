import { useCallback, useEffect, useRef, useState } from "react";

import { CommandError, toCommandError } from "@/lib/problem";

/** One card's way of running a command started by the user: one at a time, with the name of what
 *  runs (`busy`), the failure with a title that says what failed, and an inline note for a result
 *  that is not an error ("Saved."). Starting something clears the previous failure and note. */
export interface Runner {
  /** Name of what is running, `undefined` when idle. */
  busy: string | undefined;
  error: CommandError | undefined;
  /** What failed: the title of the error banner. */
  errorTitle: string;
  notice: string | undefined;
  setNotice: (text: string | undefined) => void;
  /** Run `call`; resolves to its result, or `undefined` when it failed or something else was running. */
  run: <T>(what: string, errorTitle: string, call: () => Promise<T>) => Promise<T | undefined>;
  /** Show a failure that did not come from a command. */
  fail: (message: string, title: string) => void;
  /** Run the last thing again (a failed network call). */
  retry: () => void;
  /** Forget the failure and the note. */
  reset: () => void;
}

export function useRunner(): Runner {
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<CommandError>();
  const [errorTitle, setErrorTitle] = useState("Something went wrong");
  const [notice, setNotice] = useState<string>();
  const running = useRef(false);
  const alive = useRef(true);
  const last = useRef<(() => void) | undefined>(undefined);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const run = useCallback(async <T>(what: string, title: string, call: () => Promise<T>): Promise<T | undefined> => {
    if (running.current) {
      return undefined;
    }
    running.current = true;
    last.current = () => void run(what, title, call);
    setBusy(what);
    setError(undefined);
    setNotice(undefined);
    try {
      return await call();
    } catch (thrown) {
      if (alive.current) {
        setErrorTitle(title);
        setError(toCommandError(thrown));
      }
      return undefined;
    } finally {
      running.current = false;
      if (alive.current) {
        setBusy(undefined);
      }
    }
  }, []);

  const fail = useCallback((message: string, title: string) => {
    setNotice(undefined);
    setErrorTitle(title);
    setError(new CommandError(message));
  }, []);

  const reset = useCallback(() => {
    setError(undefined);
    setNotice(undefined);
  }, []);

  const retry = useCallback(() => last.current?.(), []);

  return { busy, error, errorTitle, notice, setNotice, run, fail, retry, reset };
}
