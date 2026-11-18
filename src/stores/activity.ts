// The background jobs the sidebar lists: graph runs that are running or waiting for the user,
// whichever screen started them. Read from the shell, kept current by the graph progress events.

import { create } from "zustand";

import { onGraphProgress } from "@/api/events";
import { activity } from "@/api/shell";
import { loaded, NO_ACTIVITY, onProgress, type ActivityState } from "@/lib/activity";

/** How long the list waits after a graph step before it is read again: steps come in bursts. */
export const REREAD_MS = 200;

export const useActivity = create<ActivityState>(() => NO_ACTIVITY);

let reads = 0;

/** Read the list. Only the latest answer is kept; a failed read keeps the list shown. */
function read(): void {
  const mine = ++reads;
  activity().then(
    (items) => {
      if (mine === reads) {
        useActivity.setState((state) => loaded(state, items));
      }
    },
    (e: unknown) => console.warn("reading the background jobs failed", e),
  );
}

/** Start following the background jobs: one read, then every graph step. Returns the teardown. */
export function startActivity(): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  read();
  const stop = onGraphProgress((p) => {
    const step = onProgress(useActivity.getState(), p);
    if (step.state !== useActivity.getState()) {
      useActivity.setState(step.state);
    }
    if (step.reread) {
      clearTimeout(timer);
      timer = setTimeout(read, REREAD_MS);
    }
  });
  return () => {
    stop();
    clearTimeout(timer);
    // An answer still on its way belongs to a list nobody follows any more.
    reads++;
  };
}
