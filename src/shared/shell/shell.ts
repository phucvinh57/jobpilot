// The app shell's own state: setup (health line, banner), navigation badges, appearance, and
// the one way a link leaves the app.

import { create } from "zustand";

import { invoke, onDataChanged, onGraphProgress, type GraphProgress } from "../bridge";
import type { Problem } from "../problem/problem";
import { applyTheme, cachedAppearance, parseAppearance, watchSystem, type Appearance } from "../theme/theme";
import { useEffect, useRef } from "react";

export interface HealthLine {
  ok: boolean;
  text: string;
  /** The Settings section that helps with the first problem. */
  focus: string | null;
}

export interface Blocker {
  id: string;
  title: string;
  problem: Problem;
}

export interface ShellStatus {
  health: HealthLine;
  blockers: Blocker[];
  prefsSaved: boolean;
  /** A profile was read from the user's documents (Settings > About you). */
  profileReady: boolean;
  newLeads: number;
  applications: number;
}

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
    invoke<ShellStatus>("shared_status").then(
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
      await invoke<void>("shared_set_appearance", { appearance });
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
  invoke<string>("shared_appearance").then(
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

/** Open an http(s) link in the system browser (the webview itself never navigates). */
export function openUrl(url: string): Promise<void> {
  return invoke<void>("shared_open_url", { url });
}

/** Call `handler` for every step of every graph run (discovery, trends, prepare-CV, chat). */
export function useGraphProgress(handler: (progress: GraphProgress) => void): void {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => onGraphProgress((p) => ref.current(p)), []);
}

export type { GraphProgress };
