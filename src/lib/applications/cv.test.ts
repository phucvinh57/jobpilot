import { describe, expect, it } from "vitest";

import { canAsk, editorNote, emptyEditorText, previewPlaceholder, requirementLabel, showTraced } from "./cv";

describe("the editor note", () => {
  const idle = { busy: false, hasCv: true, rendering: false, dirty: false };

  it("says nothing when nothing happens", () => {
    expect(editorNote(idle)).toBeNull();
  });

  it("locks while the AI works, then renders, then edits", () => {
    expect(editorNote({ ...idle, busy: true, rendering: true, dirty: true })).toBe("locked while the AI works");
    expect(editorNote({ ...idle, rendering: true, dirty: true })).toBe("rendering...");
    expect(editorNote({ ...idle, dirty: true })).toBe("editing...");
  });

  it("does not claim a lock when there is no CV to lock", () => {
    expect(editorNote({ ...idle, busy: true, hasCv: false })).toBeNull();
  });
});

describe("the empty states", () => {
  it("tell a CV being written from a missing one", () => {
    expect(emptyEditorText(true)).toBe("The CV is being written...");
    expect(emptyEditorText(false)).toBe("No CV yet: Generate creates one from the profile.");
    expect(previewPlaceholder(true)).toBe("Rendering...");
    expect(previewPlaceholder(false)).toBe("No preview yet.");
  });
});

describe("Every fact traced", () => {
  const ok = { guardClean: true, hasCv: true, hasProblem: false, warnings: 0, busy: false };

  it("shows only for a CV the fact guard checked and found clean", () => {
    expect(showTraced(ok)).toBe(true);
    expect(showTraced({ ...ok, guardClean: false })).toBe(false);
    expect(showTraced({ ...ok, hasCv: false })).toBe(false);
    expect(showTraced({ ...ok, hasProblem: true })).toBe(false);
    expect(showTraced({ ...ok, warnings: 1 })).toBe(false);
    expect(showTraced({ ...ok, busy: true })).toBe(false);
  });
});

describe("the checklist and Ask AI", () => {
  it("labels a row by whether the profile covers it", () => {
    expect(requirementLabel(true)).toBe("Covered");
    expect(requirementLabel(false)).toBe("Not in your profile");
  });

  it("asks only with a CV, an instruction and no running job", () => {
    expect(canAsk({ busy: false, hasCv: true, instruction: "shorter" })).toBe(true);
    expect(canAsk({ busy: true, hasCv: true, instruction: "shorter" })).toBe(false);
    expect(canAsk({ busy: false, hasCv: false, instruction: "shorter" })).toBe(false);
    expect(canAsk({ busy: false, hasCv: true, instruction: "   " })).toBe(false);
  });
});
