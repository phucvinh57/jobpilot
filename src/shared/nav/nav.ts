// Navigation: which screen shows, which Application or Lead is open, and what the Today badge
// says. One store for the whole window; contexts use it to send the user somewhere
// ("Prepare CV" opens the Application, "Next job" goes back to Today).

import { create } from "zustand";

export type Screen = "today" | "pipeline" | "application" | "trends" | "settings";

/** The screens an Application can be opened from. */
export type Origin = "today" | "pipeline";

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

/** The screen an Application opened from `current` came from: Today or the Pipeline when it was
 *  opened from one of them, else the one before. */
export function detailOrigin(current: Screen, previous: Origin): Origin {
  return current === "today" || current === "pipeline" ? current : previous;
}

/** The navigation entry to highlight: an open Application belongs to the screen it was opened from. */
export function navScreen(screen: Screen, detailFrom: Origin): Screen {
  return screen === "application" ? detailFrom : screen;
}

export type NavBadge = { kind: "new" | "needsYou" | "total"; count: number };

/** The badge of the Today entry: what needs the user, else the number of new Leads. */
export function todayBadge(needsYou: number, newLeads: number): NavBadge | undefined {
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
  /** The open Application. */
  applicationId: number | undefined;
  /** The screen the open Application was opened from: highlighted in the sidebar, and where its breadcrumb leads. */
  detailFrom: Origin;
  /** The Lead open on Today. */
  leadId: number | undefined;
  /** The Settings section to scroll to the next time Settings is shown. */
  settingsFocus: SettingsSection | undefined;
  /** What waits for the user on Today (its "NEEDS YOU" header); written by Today. */
  needsYou: number;
  /** A file dropped on the window, waiting for Today to turn it into a Lead. */
  droppedFile: string | undefined;

  show: (screen: Screen) => void;
  openApplication: (id: number) => void;
  /** Show Today with `id` open (`undefined`: its best Lead). */
  openLead: (id: number | undefined) => void;
  openSettings: (section?: SettingsSection) => void;
  clearSettingsFocus: () => void;
  setNeedsYou: (count: number) => void;
  dropFile: (path: string) => void;
  /** Read the dropped file once. */
  takeDroppedFile: () => string | undefined;
}

export const useNav = create<NavState>((set, get) => ({
  screen: "today",
  applicationId: undefined,
  detailFrom: "pipeline",
  leadId: undefined,
  settingsFocus: undefined,
  needsYou: 0,
  droppedFile: undefined,

  show: (screen) => set({ screen }),
  openApplication: (id) =>
    set((s) => ({ screen: "application", applicationId: id, detailFrom: detailOrigin(s.screen, s.detailFrom) })),
  openLead: (id) => set({ screen: "today", leadId: id }),
  openSettings: (section) => set({ screen: "settings", settingsFocus: section }),
  clearSettingsFocus: () => set({ settingsFocus: undefined }),
  setNeedsYou: (count) => {
    if (get().needsYou !== count) {
      set({ needsYou: count });
    }
  },
  dropFile: (path) => set({ droppedFile: path, screen: "today" }),
  takeDroppedFile: () => {
    const path = get().droppedFile;
    if (path !== undefined) {
      set({ droppedFile: undefined });
    }
    return path;
  },
}));
