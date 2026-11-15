// The drawer hands the full Application screen where to start: "Review & Submit" pressed in the
// drawer opens the full view on its Apply tab and starts the form run there (the review needs it).

import type { Tab } from "@/lib/applications/tabs";

export interface FullViewStart {
  id: number;
  tab: Tab;
  /** Start "Review & Submit" once the screen is up. */
  prepare: boolean;
}

let pending: FullViewStart | undefined;

/** The next full view of application `start.id` starts like this. */
export function requestFullView(start: FullViewStart): void {
  pending = start;
}

/** How the full view of `id` starts (`undefined`: as usual), without using it up. */
export function peekFullView(id: number): FullViewStart | undefined {
  return pending?.id === id ? pending : undefined;
}

/** Use up the start of `id`: `true` when there was one. */
export function takeFullView(id: number): boolean {
  if (pending?.id !== id) {
    return false;
  }
  pending = undefined;
  return true;
}
