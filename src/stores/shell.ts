// The app's own state: setup (health line, banner), navigation badges and appearance.

import { create } from "zustand";

import { onDataChanged } from "@/api/bridge";
import { savedAppearance, saveAppearance, shellStatus } from "@/api/shell";
import { applyTheme, cachedAppearance, parseAppearance, watchSystem, type Appearance } from "@/lib/theme";
import type { ShellStatus } from "@/types/shell";

interface ShellState {
  /** `undefined` until the first check finished: nothing is claimed before it. */
  status: ShellStatus | undefined;
  appearance: Appearance;
  /** Read the setup state and the badges again. Only the latest answer is kept. */
  refresh: () => void;
  setAppearance: (appearance: Appearance) => Promise<void>;
}

let reads = 0;

export const useShell = create<ShellState>((set, get) => ({
  status: undefined,
  appearance: cachedAppearance(),
  refresh: () => {
    const read = ++reads;
    shellStatus().then(
      (status) => {
        if (read === reads) {
          set({ status });
        }
      },
      (e: unknown) => console.warn("reading the setup state failed", e),
    );
  },
  setAppearance: async (appearance) => {
    const before = get().appearance;
    set({ appearance });
    applyTheme(appearance);
    try {
      await saveAppearance(appearance);
    } catch (e) {
      set({ appearance: before });
      applyTheme(before);
      throw e;
    }
  },
}));

/** Start the shell: theme, first status read, and the subscriptions that keep both current.
 *  Returns the teardown. */
export function startShell(): () => void {
  applyTheme(useShell.getState().appearance);
  savedAppearance().then(
    (saved) => {
      const appearance = parseAppearance(saved);
      useShell.setState({ appearance });
      applyTheme(appearance);
    },
    (e: unknown) => console.warn("reading the appearance failed", e),
  );
  useShell.getState().refresh();
  const stopSystem = watchSystem(() => applyTheme(useShell.getState().appearance));
  // Anything stored may move a badge or fix a prerequisite.
  const stopData = onDataChanged(() => useShell.getState().refresh());
  return () => {
    stopSystem();
    stopData();
  };
}
