// Navigation: which screen shows, which stage of the Jobs page, what its drawer holds, and what
// the Jobs badge says. One store for the whole window; contexts use it to send the user somewhere
// ("Prepare CV" opens the new application in the drawer, "Next job" opens the best Lead).

import { create } from "zustand";

export type Screen = "jobs" | "application" | "trends" | "settings";

/** The stages of the Jobs page, in the order a job moves through them. */
export type Stage = "leads" | "cv" | "applied" | "inProgress" | "done";

export const STAGES: readonly Stage[] = ["leads", "cv", "applied", "inProgress", "done"];

/** What the Jobs drawer shows: a Lead (`"best"`: the best one shown) or an application. */
export type DrawerTarget = { kind: "lead"; id: number | "best" } | { kind: "app"; id: number };

/** The Settings sections, by the title the shell's `Problem.settingsSection` uses. */
export type SettingsSection =
  | "Profile & preferences"
  | "AI providers"
  | "Browser sessions"
  | "Job sources"
  | "Gmail"
  | "Appearance";

/** The Settings sections in display order. */
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  "Profile & preferences",
  "AI providers",
  "Gmail",
  "Browser sessions",
  "Job sources",
  "Appearance",
];

/** The DOM id of a Settings section (`settings-ai-providers`): what a call to action scrolls to. */
export function sectionId(section: SettingsSection): string {
  return `settings-${section.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;
}

// ------------------------------------------------------------------ rules

/** The navigation entry to highlight: an open Application belongs to the Jobs page. */
export function navScreen(screen: Screen): Screen {
  return screen === "application" ? "jobs" : screen;
}

export type NavBadge = { kind: "new" | "needsYou" | "total"; count: number };

/** The badge of the Jobs entry: what needs the user, else the number of new Leads. */
export function jobsBadge(needsYou: number, newLeads: number): NavBadge | undefined {
  if (needsYou > 0) {
    return { kind: "needsYou", count: needsYou };
  }
  if (newLeads > 0) {
    return { kind: "new", count: newLeads };
  }
  return undefined;
}

/** The badge as assistive technology reads it after the label. */
export function spokenBadge(badge: NavBadge): string {
  switch (badge.kind) {
    case "new":
      return `${badge.count} new`;
    case "needsYou":
      return `${badge.count} need you`;
    case "total":
      return String(badge.count);
  }
}

// ------------------------------------------------------------------ store

export interface NavState {
  screen: Screen;
  /** The stage shown on the Jobs page. */
  stage: Stage;
  /** What the Jobs drawer shows (`undefined`: closed). */
  drawer: DrawerTarget | undefined;
  /** The Application open full screen. */
  applicationId: number | undefined;
  /** The Settings section to scroll to the next time Settings is shown. */
  settingsFocus: SettingsSection | undefined;
  /** What waits for the user on the Jobs page (its "NEEDS YOU" header); written by discovery. */
  needsYou: number;
  /** A file dropped on the window, waiting for the Jobs page to turn it into a Lead. */
  droppedFile: string | undefined;

  show: (screen: Screen) => void;
  /** Show the Jobs page at `stage`, with the drawer closed. */
  showStage: (stage: Stage) => void;
  /** Show Lead `id` in the drawer of the Leads stage (`"best"`: the best Lead shown). */
  openLead: (id: number | "best") => void;
  /** Show application `id` in the drawer, at `stage` when given. */
  openAppDrawer: (id: number, stage?: Stage) => void;
  closeDrawer: () => void;
  /** Open application `id` full screen (the drawer's "Open full view"). */
  openApplication: (id: number) => void;
  openSettings: (section?: SettingsSection) => void;
  clearSettingsFocus: () => void;
  setNeedsYou: (count: number) => void;
  dropFile: (path: string) => void;
  /** Read the dropped file once. */
  takeDroppedFile: () => string | undefined;
}

export const useNav = create<NavState>((set, get) => ({
  screen: "jobs",
  stage: "leads",
  drawer: undefined,
  applicationId: undefined,
  settingsFocus: undefined,
  needsYou: 0,
  droppedFile: undefined,

  show: (screen) => set({ screen }),
  showStage: (stage) => set({ screen: "jobs", stage, drawer: undefined }),
  openLead: (id) => set({ screen: "jobs", stage: "leads", drawer: { kind: "lead", id } }),
  openAppDrawer: (id, stage) => set((s) => ({ screen: "jobs", stage: stage ?? s.stage, drawer: { kind: "app", id } })),
  closeDrawer: () => set({ drawer: undefined }),
  openApplication: (id) => set({ screen: "application", applicationId: id }),
  openSettings: (section) => set({ screen: "settings", settingsFocus: section }),
  clearSettingsFocus: () => set({ settingsFocus: undefined }),
  setNeedsYou: (count) => {
    if (get().needsYou !== count) {
      set({ needsYou: count });
    }
  },
  dropFile: (path) => set({ droppedFile: path, screen: "jobs", stage: "leads" }),
  takeDroppedFile: () => {
    const path = get().droppedFile;
    if (path !== undefined) {
      set({ droppedFile: undefined });
    }
    return path;
  },
}));
