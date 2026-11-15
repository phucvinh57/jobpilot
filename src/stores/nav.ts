// Navigation: which screen shows, which stage of the Jobs page, what its drawer holds, and what
// the Jobs badge says. One store for the whole window; screens use it to send the user somewhere
// ("Prepare CV" opens the new application in the drawer, "Next job" opens the best Lead).

import { create } from "zustand";

export type Screen = "jobs" | "application" | "trends" | "profile" | "settings";

/** The stages of the Jobs page, in the order a job moves through them. */
export type Stage = "leads" | "cv" | "applied" | "inProgress" | "done";

export const STAGES: readonly Stage[] = ["leads", "cv", "applied", "inProgress", "done"];

/** What the Jobs drawer shows: a Lead (`"best"`: the best one shown) or an application. */
export type DrawerTarget = { kind: "lead"; id: number | "best" } | { kind: "app"; id: number };

/** The Settings sections, by the title the shell's `Problem.settingsSection` uses. */
export type SettingsSection = "AI providers" | "Browser sessions" | "Job sources" | "Gmail" | "Appearance";

/** The Settings sections in display order. */
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  "AI providers",
  "Gmail",
  "Browser sessions",
  "Job sources",
  "Appearance",
];

/** The Profile sections: the user's documents and the profile read from them, then what to look
 *  for. "About you" is also the title the shell's `Problem.settingsSection` uses. */
export type ProfileSection = "About you" | "Preferences";

/** The Profile sections in display order. */
export const PROFILE_SECTIONS: readonly ProfileSection[] = ["About you", "Preferences"];

function isProfileSection(name: string): name is ProfileSection {
  return (PROFILE_SECTIONS as readonly string[]).includes(name);
}

function isSettingsSection(name: string): name is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(name);
}

/** The DOM id of a section (`settings-ai-providers`, `profile-about-you`): what a call to action
 *  scrolls to. */
export function sectionId(section: SettingsSection | ProfileSection): string {
  const page = isProfileSection(section) ? "profile" : "settings";
  return `${page}-${section.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;
}

/** The Profile section a name stands for ("About you", "Preferences", and the old card title
 *  "Profile & preferences"); `undefined` for every other name (a Settings section). */
export function profileSectionOf(name: string): ProfileSection | undefined {
  if (name === "Profile & preferences") {
    return "Preferences";
  }
  return isProfileSection(name) ? name : undefined;
}

/** The page a section is on, for a call to action's label ("Open Profile > About you"). */
export function sectionPlace(name: string): "Profile" | "Settings" {
  return profileSectionOf(name) ? "Profile" : "Settings";
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
  /** The Profile section to scroll to the next time Profile is shown. */
  profileFocus: ProfileSection | undefined;
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
  openProfile: (section?: ProfileSection) => void;
  clearProfileFocus: () => void;
  /** Open the page that holds section `name` (a `Problem.settingsSection`, a primary's
   *  `settings`): About you and Preferences are on Profile, everything else on Settings. */
  openSection: (name: string) => void;
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
  profileFocus: undefined,
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
  openProfile: (section) => set({ screen: "profile", profileFocus: section }),
  clearProfileFocus: () => set({ profileFocus: undefined }),
  openSection: (name) => {
    const profile = profileSectionOf(name);
    if (profile) {
      set({ screen: "profile", profileFocus: profile });
    } else {
      // A name this app does not know still opens Settings, at the top.
      set({ screen: "settings", settingsFocus: isSettingsSection(name) ? name : undefined });
    }
  },
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
