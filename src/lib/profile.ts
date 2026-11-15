// The rules only Profile > Preferences needs: when Save is on, when a "Saved" label
// shows and how it fades, when the notes box follows the stored notes. The rules about list
// entries (clean, compare, refuse) live in the shell.

import type { JobType, Profile } from "@/types/profile";

export const JOB_TYPES: readonly { value: JobType; label: string }[] = [
  { value: "any", label: "Any" },
  { value: "remote", label: "Remote" },
  { value: "hybrid", label: "Hybrid" },
  { value: "onsite", label: "On-site" },
];

/** What the background reading of the notes is doing. */
export type NotesStatus = { kind: "idle" } | { kind: "reading" } | { kind: "failed"; line: string };

export const NOTES_IDLE: NotesStatus = { kind: "idle" };

// ------------------------------------------------------------------ the "Saved" label

/** How long the "Saved" label stays before it is gone, in milliseconds. */
export const FLASH_MS = 2200;
/** Share of that time the label stays fully visible before it fades. */
export const FLASH_HOLD = 0.55;

/** Opacity of the "Saved" label at `delta` (0 to 1 over its life): fully visible, then gone. */
export function fade(delta: number): number {
  if (delta <= FLASH_HOLD) {
    return 1;
  }
  return Math.min(1, Math.max(0, 1 - (delta - FLASH_HOLD) / (1 - FLASH_HOLD)));
}

/** An answer that shows `next` is worth a "Saved" label only when it differs from what is shown
 *  (`shown`): an edit that changed nothing saved nothing. */
export function changed(shown: Profile | undefined, next: Profile): boolean {
  if (shown === undefined) {
    return true;
  }
  return (
    shown.jobType !== next.jobType ||
    shown.notes !== next.notes ||
    shown.fields.length !== next.fields.length ||
    shown.fields.some((field, i) => {
      const other = next.fields[i];
      return (
        other === undefined ||
        field.items.length !== other.items.length ||
        field.items.some((item, j) => item !== other.items[j])
      );
    })
  );
}

// ------------------------------------------------------------------ the notes

/** Notes as they are saved: trimmed and cut to `max` characters (code points, like the core). */
export function cleanNotes(raw: string, max: number): string {
  return Array.from(raw.trim()).slice(0, max).join("");
}

/** What Save does with the notes. */
export type SaveNotes =
  /** Nothing changed and nothing failed. */
  | "nothing"
  /** The text is saved already but the model could not read it last time: read it again. */
  | "readAgain"
  /** New text: save it, then let the model read it. */
  | "saveAndRead";

/** `text` is what the box holds (cleaned), `saved` what is stored. */
export function savePlan(text: string, saved: string, lastReadFailed: boolean): SaveNotes {
  if (text !== saved) {
    return "saveAndRead";
  }
  return lastReadFailed && text !== "" ? "readAgain" : "nothing";
}

/** The notes box follows the stored notes unless the user has typed since it was last synced:
 *  the text to put into the box, or `undefined` to leave it alone. */
export function followNotes(typed: string, synced: string, saved: string, max: number): string | undefined {
  const clean = cleanNotes(typed, max);
  return clean === cleanNotes(synced, max) && clean !== saved ? saved : undefined;
}

/** The shortcut that saves the notes from the box. */
export function saveShortcut(platform: string): string {
  return /mac/i.test(platform) ? "Cmd+Enter" : "Ctrl+Enter";
}
