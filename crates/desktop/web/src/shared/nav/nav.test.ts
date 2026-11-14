import { beforeEach, describe, expect, it } from "vitest";

import { detailOrigin, navScreen, sectionId, SETTINGS_SECTIONS, spokenBadge, todayBadge, useNav } from "./nav";

describe("navigation rules", () => {
  it("an application remembers where it was opened from", () => {
    expect(detailOrigin("today", "pipeline")).toBe("today");
    expect(detailOrigin("pipeline", "today")).toBe("pipeline");
    expect(detailOrigin("application", "today")).toBe("today");
    expect(detailOrigin("settings", "pipeline")).toBe("pipeline");
    expect(detailOrigin("trends", "today")).toBe("today");
  });

  it("an open application highlights where it was opened from", () => {
    expect(navScreen("application", "today")).toBe("today");
    expect(navScreen("application", "pipeline")).toBe("pipeline");
    expect(navScreen("trends", "today")).toBe("trends");
    expect(navScreen("settings", "pipeline")).toBe("settings");
  });

  it("today shows what needs you before new leads", () => {
    expect(todayBadge(2, 5)).toEqual({ kind: "needsYou", count: 2 });
    expect(todayBadge(0, 5)).toEqual({ kind: "new", count: 5 });
    expect(todayBadge(1, 0)).toEqual({ kind: "needsYou", count: 1 });
    expect(todayBadge(0, 0)).toBeUndefined();
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

  it("opens an application from the current list screen", () => {
    useNav.getState().show("pipeline");
    useNav.getState().openApplication(7);
    expect(useNav.getState()).toMatchObject({ screen: "application", applicationId: 7, detailFrom: "pipeline" });
    useNav.getState().openApplication(8);
    expect(useNav.getState().detailFrom).toBe("pipeline");
  });

  it("opens Settings at a section", () => {
    useNav.getState().openSettings("AI providers");
    expect(useNav.getState()).toMatchObject({ screen: "settings", settingsFocus: "AI providers" });
  });

  it("hands a dropped file to Today once", () => {
    useNav.getState().show("trends");
    useNav.getState().dropFile("/tmp/jd.pdf");
    expect(useNav.getState().screen).toBe("today");
    expect(useNav.getState().takeDroppedFile()).toBe("/tmp/jd.pdf");
    expect(useNav.getState().takeDroppedFile()).toBeUndefined();
  });
});
