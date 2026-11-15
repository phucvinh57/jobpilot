// What the "Profile & preferences" card shows (the shell's `ProfileDto`).

export type FieldKey = "locations" | "domains" | "skills";

export type JobType = "any" | "remote" | "hybrid" | "onsite";

/** One tag field: Work locations, Interested domains, Skills. */
export interface ListField {
  key: FieldKey;
  label: string;
  placeholder: string;
  items: string[];
  /** Entries that could be added with one click. */
  suggestions: string[];
}

/** The CV attached for suggestions (context only: never copied into a CV). */
export interface Cv {
  name: string;
  chars: number;
  truncated: boolean;
}

export interface Limits {
  maxListItems: number;
  maxItemChars: number;
  maxNotesChars: number;
}

/** Everything the card shows, read in one go. */
export interface Profile {
  /** The master profile (`profile.yaml`) the CVs are written from. */
  profilePath: string;
  jobType: JobType;
  fields: ListField[];
  /** The user's own words, as saved. */
  notes: string;
  cv: Cv | null;
  canUndo: boolean;
  limits: Limits;
}

/** The answer of an add: the card, and why nothing was added when the user should be told. */
export interface AddResult {
  profile: Profile;
  message: string | null;
}

/** The answer of reading the notes: the card as the model's merge left it, and the muted line
 *  under the notes (none after a clean read). */
export interface NotesReadResult {
  profile: Profile;
  line: string | null;
}
