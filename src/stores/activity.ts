// The background jobs the sidebar lists: graph runs that are running or waiting for the user,
// whichever screen started them. Read from the shell, kept current by the graph progress events.

import { create } from "zustand";

import { onGraphProgress } from "@/api/events";
import { activity } from "@/api/shell";
import { loaded, NO_ACTIVITY, onProgress, type ActivityState } from "@/lib/activity";

/** The list is read again at most once in this long, however many graph steps arrive (they come
 *  in bursts), and never later than this after a step. */
export const REREAD_MS = 250;

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

/** Start following the background jobs: one read, then one after graph steps. A step asks for a
 *  read `REREAD_MS` later unless one is already due: the steps in between are covered by that read,
 *  so a steady stream of steps reads once per interval instead of putting the read off. Returns
 *  the teardown. */
export function startActivity(): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  read();
  const stop = onGraphProgress((p) => {
    const step = onProgress(useActivity.getState(), p);
    if (step.state !== useActivity.getState()) {
      useActivity.setState(step.state);
    }
    if (step.reread && timer === undefined) {
      timer = setTimeout(() => {
        timer = undefined;
        read();
      }, REREAD_MS);
    }
  });
  return () => {
    stop();
    clearTimeout(timer);
    // An answer still on its way belongs to a list nobody follows any more.
    reads++;
  };
}
