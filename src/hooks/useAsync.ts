// The two shapes of talking to the shell from a hook: read something and keep it fresh
// (`useQuery`), or do something on a click (`useAction`). Both turn a failure into a
// `CommandError`, so a view can show its `Problem`.

import { useCallback, useEffect, useRef, useState } from "react";

import { onDataChanged, type Scope } from "@/api/bridge";
import { toCommandError, type CommandError } from "@/lib/problem";

export interface Query<T> {
  /** The last answer; kept while a reload runs. */
  data: T | undefined;
  error: CommandError | undefined;
  /** The first load has not answered yet. */
  loading: boolean;
  reload: () => void;
}

/** The answer of read number `read` is the one to show only while it is the latest started. */
export function isLatestRead(read: number, latest: number): boolean {
  return read === latest;
}

/**
 * Read with `load`, again whenever `deps` change, `reload()` is called or one of `refreshOn`
 * changed. An older answer that arrives after a newer one is dropped.
 */
export function useQuery<T>(
  load: () => Promise<T>,
  deps: readonly unknown[],
  refreshOn: readonly Scope[] = [],
): Query<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<CommandError>();
  const [loading, setLoading] = useState(true);
  const reads = useRef(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  const reload = useCallback(() => {
    const read = ++reads.current;
    loadRef.current().then(
      (value) => {
        if (isLatestRead(read, reads.current)) {
          setData(value);
          setError(undefined);
          setLoading(false);
        }
      },
      (thrown: unknown) => {
        if (isLatestRead(read, reads.current)) {
          setError(toCommandError(thrown));
          setLoading(false);
        }
      },
    );
  }, []);

  useEffect(reload, [reload, ...deps]);

  const scopes = refreshOn.join(",");
  useEffect(() => {
    if (!scopes) {
      return;
    }
    const wanted = new Set(scopes.split(","));
    return onDataChanged((scope) => {
      if (wanted.has(scope)) {
        reload();
      }
    });
  }, [scopes, reload]);

  // Answers that arrive after the component is gone are dropped.
  useEffect(
    () => () => {
      // One more read "started": every answer still in flight is stale. (Adding one, never a
      // sentinel: a remount keeps counting from here.)
      reads.current += 1;
    },
    [],
  );

  return { data, error, loading, reload };
}

export interface Action<Args extends unknown[], T> {
  /** Run it. Resolves to the result, or `undefined` when it failed (see `error`) or was already running. */
  run: (...args: Args) => Promise<T | undefined>;
  busy: boolean;
  error: CommandError | undefined;
  clearError: () => void;
}

/** A command started by the user. One run at a time: a second call while busy does nothing. */
export function useAction<Args extends unknown[], T>(perform: (...args: Args) => Promise<T>): Action<Args, T> {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError>();
  const running = useRef(false);
  const alive = useRef(true);
  const performRef = useRef(perform);
  performRef.current = perform;

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const run = useCallback(async (...args: Args): Promise<T | undefined> => {
    if (running.current) {
      return undefined;
    }
    running.current = true;
    setBusy(true);
    setError(undefined);
    try {
      return await performRef.current(...args);
    } catch (thrown) {
      if (alive.current) {
        setError(toCommandError(thrown));
      }
      return undefined;
    } finally {
      running.current = false;
      if (alive.current) {
        setBusy(false);
      }
    }
  }, []);

  const clearError = useCallback(() => setError(undefined), []);
  return { run, busy, error, clearError };
}
