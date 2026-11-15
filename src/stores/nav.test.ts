import { beforeEach, describe, expect, it } from "vitest";

import {
  jobsBadge,
  navScreen,
  PROFILE_SECTIONS,
  profileSectionOf,
  sectionId,
  sectionPlace,
  SETTINGS_SECTIONS,
  spokenBadge,
  useNav,
} from "./nav";

describe("navigation rules", () => {
  it("an open application highlights the Jobs page", () => {
    expect(navScreen("application")).toBe("jobs");
    expect(navScreen("trends")).toBe("trends");
    expect(navScreen("profile")).toBe("profile");
    expect(navScreen("settings")).toBe("settings");
  });

  it("jobs shows what needs you before new leads", () => {
    expect(jobsBadge(2, 5)).toEqual({ kind: "needsYou", count: 2 });
    expect(jobsBadge(0, 5)).toEqual({ kind: "new", count: 5 });
    expect(jobsBadge(1, 0)).toEqual({ kind: "needsYou", count: 1 });
    expect(jobsBadge(0, 0)).toBeUndefined();
  });

  it("badges are spoken after the label", () => {
    expect(spokenBadge({ kind: "new", count: 3 })).toBe("3 new");
    expect(spokenBadge({ kind: "needsYou", count: 2 })).toBe("2 need you");
    expect(spokenBadge({ kind: "total", count: 8 })).toBe("8");
  });
});

describe("settings and profile sections", () => {
  it("a call to action finds its card", () => {
    expect(sectionId("AI providers")).toBe("settings-ai-providers");
    expect(sectionId("About you")).toBe("profile-about-you");
    expect(sectionId("Preferences")).toBe("profile-preferences");
    expect(SETTINGS_SECTIONS[0]).toBe("AI providers");
    expect(PROFILE_SECTIONS).toEqual(["About you", "Preferences"]);
    const ids = [...SETTINGS_SECTIONS, ...PROFILE_SECTIONS].map(sectionId);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("About you and Preferences are on Profile, the rest on Settings", () => {
    expect(sectionPlace("About you")).toBe("Profile");
    expect(sectionPlace("Preferences")).toBe("Profile");
    expect(sectionPlace("Profile & preferences")).toBe("Profile");
    expect(profileSectionOf("Profile & preferences")).toBe("Preferences");
    for (const section of SETTINGS_SECTIONS) {
      expect(sectionPlace(section)).toBe("Settings");
      expect(profileSectionOf(section)).toBeUndefined();
    }
    expect(sectionPlace("Something new")).toBe("Settings");
  });
});

describe("navigation store", () => {
  beforeEach(() => useNav.setState(useNav.getInitialState()));

  it("a stage opens with the drawer closed", () => {
    useNav.getState().openLead(3);
    useNav.getState().showStage("cv");
    expect(useNav.getState()).toMatchObject({ screen: "jobs", stage: "cv", drawer: undefined });
  });

  it("a lead opens in the drawer of the Leads stage", () => {
    useNav.getState().show("trends");
    useNav.getState().openLead(4);
    expect(useNav.getState()).toMatchObject({ screen: "jobs", stage: "leads", drawer: { kind: "lead", id: 4 } });
    useNav.getState().closeDrawer();
    expect(useNav.getState().drawer).toBeUndefined();
  });

  it("an application opens in the drawer, then full screen, and back keeps the drawer", () => {
    useNav.getState().openAppDrawer(7, "cv");
    expect(useNav.getState()).toMatchObject({ screen: "jobs", stage: "cv", drawer: { kind: "app", id: 7 } });
    useNav.getState().openApplication(7);
    expect(useNav.getState()).toMatchObject({ screen: "application", applicationId: 7 });
    useNav.getState().show("jobs");
    expect(useNav.getState()).toMatchObject({ stage: "cv", drawer: { kind: "app", id: 7 } });
  });

  it("opens Settings at a section", () => {
    useNav.getState().openSettings("AI providers");
    expect(useNav.getState()).toMatchObject({ screen: "settings", settingsFocus: "AI providers" });
  });

  it("opens Profile at a section", () => {
    useNav.getState().openProfile("Preferences");
    expect(useNav.getState()).toMatchObject({ screen: "profile", profileFocus: "Preferences" });
    useNav.getState().clearProfileFocus();
    expect(useNav.getState().profileFocus).toBeUndefined();
  });

  it("opens the page a section is on", () => {
    // The shell names "About you" for a missing profile (`Problem.settingsSection`).
    useNav.getState().openSection("About you");
    expect(useNav.getState()).toMatchObject({ screen: "profile", profileFocus: "About you" });
    useNav.getState().openSection("Profile & preferences");
    expect(useNav.getState()).toMatchObject({ screen: "profile", profileFocus: "Preferences" });
    useNav.getState().openSection("Browser sessions");
    expect(useNav.getState()).toMatchObject({ screen: "settings", settingsFocus: "Browser sessions" });
    useNav.getState().openSection("Something new");
    expect(useNav.getState()).toMatchObject({ screen: "settings", settingsFocus: undefined });
  });

  it("hands a dropped file to the Leads stage once", () => {
    useNav.getState().show("trends");
    useNav.getState().dropFile("/tmp/jd.pdf");
    expect(useNav.getState()).toMatchObject({ screen: "jobs", stage: "leads" });
    expect(useNav.getState().takeDroppedFile()).toBe("/tmp/jd.pdf");
    expect(useNav.getState().takeDroppedFile()).toBeUndefined();
  });
});
