// What the user types in the review of a filled form, and how a row's answer changes.
// The review shows the answers; the shell keeps a copy (`applications_apply_sync`) so the primary
// button knows whether required fields are still empty.

import type { ApplyReview, ReviewEdits, ReviewRow } from "@/types/applications";

/** A file field is attached by the run, never typed. */
function isTyped(row: ReviewRow): boolean {
  return row.editable && row.kind !== "file";
}

/** What a row has now: what the user typed, else what the form was filled with. */
export function fieldValue(edits: ReviewEdits, row: ReviewRow): string {
  return edits.values[row.id] ?? row.value;
}

/** The edits a new review starts from: the filled value of every editable row, nothing remembered
 *  except what the shell already marked. */
export function initialEdits(review: ApplyReview | null): ReviewEdits {
  if (review === null) {
    return { values: {}, remember: [] };
  }
  const values: Record<string, string> = {};
  const remember: string[] = [];
  for (const row of review.rows) {
    if (row.editable) {
      values[row.id] = row.value;
    }
    if (row.remember) {
      remember.push(row.id);
    }
  }
  return { values, remember };
}

/** The answer of one row changed. */
export function withValue(edits: ReviewEdits, id: string, value: string): ReviewEdits {
  return { ...edits, values: { ...edits.values, [id]: value } };
}

/** The "Remember this answer" box of one row was toggled. */
export function toggleRemember(edits: ReviewEdits, id: string): ReviewEdits {
  const remember = edits.remember.includes(id) ? edits.remember.filter((r) => r !== id) : [...edits.remember, id];
  return { ...edits, remember };
}

/** A required answer that is still empty (files are attached by the run). */
export function isEmptyRequired(row: ReviewRow, value: string): boolean {
  return isTyped(row) && row.required && value.trim() === "";
}

/** Required rows that are still empty, from what is typed right now. */
export function missingCount(review: ApplyReview, edits: ReviewEdits): number {
  return review.rows.filter((row) => isEmptyRequired(row, fieldValue(edits, row))).length;
}

/** The answers the shell gets: editable rows only. */
export function snapshot(review: ApplyReview, edits: ReviewEdits): ReviewEdits {
  const values: Record<string, string> = {};
  for (const row of review.rows) {
    if (row.editable) {
      values[row.id] = fieldValue(edits, row);
    }
  }
  return { values, remember: edits.remember.filter((id) => review.rows.some((r) => r.id === id && r.editable)) };
}

/** A row is highlighted when it needs a look or a required answer is missing. */
export function isHighlighted(row: ReviewRow, value: string): boolean {
  return row.flagged || isEmptyRequired(row, value);
}

/** "Yes" boxes store `yes` / `no`. */
export function checkboxValue(checked: boolean): "yes" | "no" {
  return checked ? "yes" : "no";
}

export function isChecked(value: string): boolean {
  return value === "yes";
}

/** The options a choice row has now: one for a single choice, any number for a group. */
export function chosenOptions(multi: boolean, value: string): string[] {
  if (multi) {
    return value
      .split(";")
      .map((s) => s.trim())
      .filter((s) => s !== "");
  }
  return [value];
}

function sameOption(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

export function isChosen(chosen: readonly string[], option: string): boolean {
  return chosen.some((c) => sameOption(c, option));
}

/** The value after a click on `option`: a group toggles it, a single choice picks it (or clears it
 *  when it was the one chosen). */
export function toggleChoice(multi: boolean, value: string, option: string): string {
  const chosen = chosenOptions(multi, value);
  if (multi) {
    return isChosen(chosen, option)
      ? chosen.filter((c) => !sameOption(c, option)).join("; ")
      : [...chosen, option].join("; ");
  }
  return isChosen(chosen, option) ? "" : option;
}

/** The "Remember" box shows for answers that can go to the profile and are not empty. */
export function canRemember(row: ReviewRow, value: string): boolean {
  return row.rememberable && value.trim() !== "";
}

/** What a read-only row shows. */
export function readOnlyText(row: ReviewRow): string {
  return row.shown === "" ? "(empty)" : row.shown;
}

/** The label of a row: required ones end with a star. */
export function rowLabel(row: ReviewRow): string {
  return row.required ? `${row.label} *` : row.label;
}

/** The tag of a row that is not on this step. */
export function stepTag(row: ReviewRow): string | null {
  return row.editable ? null : `step ${row.step + 1}`;
}
