// Typed calls over `profile_*` commands. A call that changes the stored preferences or the documents
// tells every screen (`profile`), so the setup banner ("Tell JobPilot about your work", "... what job
// you are looking for") follows.

import { invoke, notifyChanged, pickFile } from "./bridge";

import { DOCUMENT_EXTENSIONS } from "@/lib/profile/about";
import type { About, AddResult, FieldKey, JobType, NotesReadResult, Profile } from "@/types/profile";

/** The card: preferences, suggestions, whether there is a change to undo. */
export function readProfile(): Promise<Profile> {
  return invoke<Profile>("profile_get");
}

async function changes<T>(call: Promise<T>): Promise<T> {
  const result = await call;
  notifyChanged("profile");
  return result;
}

/** Add an entry to a list; the answer says why nothing was added when the user should know. */
export function addItem(field: FieldKey, text: string): Promise<AddResult> {
  return changes(invoke<AddResult>("profile_add_item", { field, text }));
}

/** Remove an entry from a list, by value. */
export function removeItem(field: FieldKey, item: string): Promise<Profile> {
  return changes(invoke<Profile>("profile_remove_item", { field, item }));
}

export function setJobType(jobType: JobType): Promise<Profile> {
  return changes(invoke<Profile>("profile_set_job_type", { jobType }));
}

/** Save the notes ("Anything else") as typed. */
export function saveNotes(notes: string): Promise<Profile> {
  return changes(invoke<Profile>("profile_save_notes", { notes }));
}

/** Let the model read the notes in the background (the `prefs_chat` graph); resolves when it is
 *  done, with what it merged into the preferences. */
export function readNotes(notes: string): Promise<NotesReadResult> {
  return changes(invoke<NotesReadResult>("profile_read_notes", { notes }));
}

/** Put back the preferences from before the last change. */
export function undo(): Promise<Profile> {
  return changes(invoke<Profile>("profile_undo"));
}

// ------------------------------------------------------------------ About you

/** Profile > About you: the documents, where the reading stands, what was understood. */
export function readAbout(): Promise<About> {
  return invoke<About>("profile_about");
}

/** The native file picker for a document: the chosen path, or `null` when cancelled. */
export function pickDocument(): Promise<string | null> {
  return pickFile("Choose your CV or another document (.pdf, .md or .txt)", [
    { name: "Document", extensions: [...DOCUMENT_EXTENSIONS] },
  ]);
}

/** Keep the file at `path` as a document (it is not read into the profile yet). */
export function addFile(path: string): Promise<About> {
  return changes(invoke<About>("profile_add_file", { path }));
}

/** Keep a typed note as a document (it is not read into the profile yet). */
export function addText(name: string, text: string): Promise<About> {
  return changes(invoke<About>("profile_add_text", { name, text }));
}

/** Forget a document; the last one takes the profile with it. */
export function removeSource(id: number): Promise<About> {
  return changes(invoke<About>("profile_remove_source", { id }));
}

/** Read the documents into the profile (one model call, up to a minute). */
export function readDocuments(): Promise<About> {
  return changes(invoke<About>("profile_read"));
}
