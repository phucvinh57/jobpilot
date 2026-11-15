import { describe, expect, it } from "vitest";

import {
  canRemember,
  checkboxValue,
  chosenOptions,
  initialEdits,
  isChecked,
  isChosen,
  isEmptyRequired,
  isHighlighted,
  missingCount,
  readOnlyText,
  rowLabel,
  snapshot,
  stepTag,
  toggleChoice,
  toggleRemember,
  fieldValue,
  withValue,
} from "./apply";
import type { ApplyReview, ReviewRow } from "./types";

function row(over: Partial<ReviewRow> = {}): ReviewRow {
  return {
    id: "name",
    label: "Full name",
    required: true,
    step: 0,
    editable: true,
    kind: "text",
    input: "line",
    options: [],
    placeholder: "Your answer",
    value: "Ada",
    shown: "Ada",
    tag: { label: "profile", tone: "ok" },
    warning: null,
    flagged: false,
    rememberable: true,
    remember: false,
    ...over,
  };
}

function review(rows: ReviewRow[]): ApplyReview {
  return {
    title: "Greenhouse - boards.example.com",
    subtitle: "",
    closeLabel: "Discard",
    dryRun: false,
    complete: true,
    missing: 0,
    notice: null,
    notes: [],
    rows,
    attached: null,
    screenshots: 0,
  };
}

describe("the answers of a review", () => {
  it("starts from the filled values of the editable rows and the answers the shell remembers", () => {
    const r = review([
      row(),
      row({ id: "later", editable: false, value: "x" }),
      row({ id: "pitch", remember: true, value: "hi" }),
    ]);
    expect(initialEdits(r)).toEqual({ values: { name: "Ada", pitch: "hi" }, remember: ["pitch"] });
    expect(initialEdits(null)).toEqual({ values: {}, remember: [] });
  });

  it("changes one answer without touching the others", () => {
    const edits = withValue({ values: { a: "1", b: "2" }, remember: [] }, "a", "9");
    expect(edits.values).toEqual({ a: "9", b: "2" });
    expect(fieldValue(edits, row({ id: "a" }))).toBe("9");
    expect(fieldValue(edits, row({ id: "c", value: "filled" }))).toBe("filled");
  });

  it("toggles what goes to the profile", () => {
    const on = toggleRemember({ values: {}, remember: [] }, "a");
    expect(on.remember).toEqual(["a"]);
    expect(toggleRemember(on, "a").remember).toEqual([]);
  });

  it("counts required answers that are still empty, files and read-only rows excepted", () => {
    const r = review([
      row({ id: "a", value: "" }),
      row({ id: "b", value: "x" }),
      row({ id: "cv", kind: "file", value: "" }),
      row({ id: "later", editable: false, value: "" }),
      row({ id: "opt", required: false, value: "" }),
    ]);
    expect(missingCount(r, { values: {}, remember: [] })).toBe(1);
    expect(missingCount(r, { values: { a: "now filled" }, remember: [] })).toBe(0);
    expect(missingCount(r, { values: { b: "   " }, remember: [] })).toBe(2);
    expect(isEmptyRequired(row({ value: "" }), "")).toBe(true);
    expect(isEmptyRequired(row(), "x")).toBe(false);
  });

  it("sends the editable rows only", () => {
    const r = review([row({ id: "a" }), row({ id: "later", editable: false })]);
    const sent = snapshot(r, { values: { a: "typed", later: "ignored" }, remember: ["a", "later", "ghost"] });
    expect(sent).toEqual({ values: { a: "typed" }, remember: ["a"] });
  });

  it("highlights rows that need a look or a required answer", () => {
    expect(isHighlighted(row({ flagged: true }), "x")).toBe(true);
    expect(isHighlighted(row(), "")).toBe(true);
    expect(isHighlighted(row(), "x")).toBe(false);
  });
});

describe("choices", () => {
  it("stores a checkbox as yes or no", () => {
    expect(checkboxValue(true)).toBe("yes");
    expect(checkboxValue(false)).toBe("no");
    expect(isChecked("yes")).toBe(true);
    expect(isChecked("no")).toBe(false);
    expect(isChecked("")).toBe(false);
  });

  it("splits a group value on semicolons", () => {
    expect(chosenOptions(true, "a; b ;;c")).toEqual(["a", "b", "c"]);
    expect(chosenOptions(true, "")).toEqual([]);
    expect(chosenOptions(false, "a; b")).toEqual(["a; b"]);
  });

  it("matches options ignoring case", () => {
    expect(isChosen(["Yes"], "yes")).toBe(true);
    expect(isChosen(["Yes"], "no")).toBe(false);
  });

  it("picks a single choice and clears it when it is clicked again", () => {
    expect(toggleChoice(false, "", "Yes")).toBe("Yes");
    expect(toggleChoice(false, "No", "Yes")).toBe("Yes");
    expect(toggleChoice(false, "yes", "Yes")).toBe("");
  });

  it("toggles one option of a group", () => {
    expect(toggleChoice(true, "", "A")).toBe("A");
    expect(toggleChoice(true, "A", "B")).toBe("A; B");
    expect(toggleChoice(true, "A; B", "a")).toBe("B");
    expect(toggleChoice(true, "A", "A")).toBe("");
  });
});

describe("the wording of a row", () => {
  it("stars required labels and tags rows of later steps", () => {
    expect(rowLabel(row())).toBe("Full name *");
    expect(rowLabel(row({ required: false }))).toBe("Full name");
    expect(stepTag(row())).toBeNull();
    expect(stepTag(row({ editable: false, step: 1 }))).toBe("step 2");
  });

  it("shows (empty) for a read-only row without a value", () => {
    expect(readOnlyText(row({ shown: "" }))).toBe("(empty)");
    expect(readOnlyText(row({ shown: "cv.pdf" }))).toBe("cv.pdf");
  });

  it("offers Remember for non-empty answers that can be saved", () => {
    expect(canRemember(row(), "x")).toBe(true);
    expect(canRemember(row(), " ")).toBe(false);
    expect(canRemember(row({ rememberable: false }), "x")).toBe(false);
  });
});
