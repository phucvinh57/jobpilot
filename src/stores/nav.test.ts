import { beforeEach, describe, expect, it } from "vitest";

import { jobsBadge, navScreen, sectionId, SETTINGS_SECTIONS, spokenBadge, useNav } from "./nav";

describe("navigation rules", () => {
  it("an open application highlights the Jobs page", () => {
    expect(navScreen("application")).toBe("jobs");
    expect(navScreen("trends")).toBe("trends");
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

describe("settings sections", () => {
  it("a call to action finds its card", () => {
    expect(sectionId("AI providers")).toBe("settings-ai-providers");
    expect(sectionId("Profile & preferences")).toBe("settings-profile-preferences");
    expect(new Set(SETTINGS_SECTIONS.map(sectionId)).size).toBe(SETTINGS_SECTIONS.length);
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

  it("hands a dropped file to the Leads stage once", () => {
    useNav.getState().show("trends");
    useNav.getState().dropFile("/tmp/jd.pdf");
    expect(useNav.getState()).toMatchObject({ screen: "jobs", stage: "leads" });
    expect(useNav.getState().takeDroppedFile()).toBe("/tmp/jd.pdf");
    expect(useNav.getState().takeDroppedFile()).toBeUndefined();
  });
});
