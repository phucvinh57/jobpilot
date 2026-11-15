// Wording and small rules of the CV tab.

export interface EditorState {
  /** A generate / revise job is running for this application. */
  busy: boolean;
  hasCv: boolean;
  /** A save + render is running. */
  rendering: boolean;
  /** The text changed and was not saved yet. */
  dirty: boolean;
}

/** The note next to the `cv.yaml` title. */
export function editorNote({ busy, hasCv, rendering, dirty }: EditorState): string | null {
  if (busy && hasCv) {
    return "locked while the AI works";
  }
  if (rendering) {
    return "rendering...";
  }
  if (dirty) {
    return "editing...";
  }
  return null;
}

/** What the editor's place says while there is no CV. */
export function emptyEditorText(busy: boolean): string {
  return busy ? "The CV is being written..." : "No CV yet: Generate creates one from the profile.";
}

/** What the paper shows while there are no pages. */
export function previewPlaceholder(rendering: boolean): string {
  return rendering ? "Rendering..." : "No preview yet.";
}

export interface TracedState {
  guardClean: boolean;
  hasCv: boolean;
  /** The CV does not parse or compile. */
  hasProblem: boolean;
  warnings: number;
  busy: boolean;
}

/** "Every fact traced" is green only when the fact guard ran on this very CV and found nothing. */
export function showTraced({ guardClean, hasCv, hasProblem, warnings, busy }: TracedState): boolean {
  return guardClean && hasCv && !hasProblem && warnings === 0 && !busy;
}

/** The icon label of a checklist row. */
export function requirementLabel(covered: boolean): string {
  return covered ? "Covered" : "Not in your profile";
}

/** The Ask AI box accepts an instruction when a CV exists and no job is running. */
export function canAsk(state: { busy: boolean; hasCv: boolean; instruction: string }): boolean {
  return state.hasCv && !state.busy && state.instruction.trim() !== "";
}

export const ASK_PLACEHOLDER = 'Ask AI: "shorter, stress Rust" (Enter)';
