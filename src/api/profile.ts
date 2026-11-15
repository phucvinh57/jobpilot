// Typed calls over `profile_*` commands. A call that changes the
// stored preferences, the attached CV or the profile file tells every screen (`profile`), so the
// setup banner ("Tell JobPilot what job you are looking for") goes away once preferences are saved.

import { invoke, notifyChanged, pickFile } from "./bridge";

import type { AddResult, FieldKey, JobType, NotesReadResult, Profile } from "@/lib/profile";

/** The card: preferences, suggestions, the attached CV, whether there is a change to undo. */
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

/** The native file picker for the CV: the chosen path, or `null` when cancelled. */
export function pickCv(): Promise<string | null> {
  return pickFile("Choose your CV (.pdf, .md or .txt)", [{ name: "CV", extensions: ["pdf", "md", "txt"] }]);
}

export function importCv(path: string): Promise<Profile> {
  return changes(invoke<Profile>("profile_import_cv", { path }));
}

export function removeCv(): Promise<Profile> {
  return changes(invoke<Profile>("profile_remove_cv"));
}

/** The native file picker for the master profile: the chosen path, or `null` when cancelled. */
export function pickProfileFile(): Promise<string | null> {
  return pickFile("Choose your profile (.yaml)", [{ name: "Profile", extensions: ["yaml", "yml"] }]);
}

/** Use another file as the master profile. */
export function setProfilePath(path: string): Promise<Profile> {
  return changes(invoke<Profile>("profile_set_path", { path }));
}
