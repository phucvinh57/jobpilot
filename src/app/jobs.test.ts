import { describe, expect, it } from "vitest";

import { drawerPad, stageItems } from "./jobs";

describe("the Jobs page", () => {
  it("puts the Leads first, then the application stages", () => {
    expect(
      stageItems(3, [
        { key: "cv", label: "CVs", count: 2 },
        { key: "done", label: "Done", count: 0 },
      ]),
    ).toEqual([
      { key: "leads", label: "Leads", count: 3 },
      { key: "cv", label: "CVs", count: 2 },
      { key: "done", label: "Done", count: 0 },
    ]);
  });

  it("makes room for the open drawer on wide screens", () => {
    expect(drawerPad(undefined)).toBe("");
    expect(drawerPad({ kind: "lead", id: "best" })).toBe("xl:pr-[480px]");
    expect(drawerPad({ kind: "app", id: 7 })).toBe("2xl:pr-[760px]");
  });
});
