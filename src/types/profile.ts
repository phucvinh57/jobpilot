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

export interface Limits {
  maxListItems: number;
  maxItemChars: number;
  maxNotesChars: number;
}

/** Everything the card shows, read in one go. */
export interface Profile {
  jobType: JobType;
  fields: ListField[];
  /** The user's own words, as saved. */
  notes: string;
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

// What Settings > About you shows (the shell's `AboutDto`).

/** One document. Never its text. */
export interface Source {
  id: number;
  /** File name or the note's title. */
  name: string;
  kind: "file" | "text";
  /** Characters kept. */
  chars: number;
  /** The file had more text than was kept. */
  truncated: boolean;
}

export interface Role {
  position: string;
  company: string;
  /** `start – end`, one side, or empty. */
  dates: string;
}

export interface Education {
  degree: string;
  institution: string;
  dates: string;
}

/** What was understood from the documents. */
export interface Summary {
  name: string;
  headline: string;
  roles: Role[];
  education: Education[];
  skills: string[];
  /** `Language (fluency)`. */
  languages: string[];
  projects: string[];
  certifications: string[];
}

/** `empty`: no documents; `unread`: documents, no profile yet; `stale`: the documents changed
 *  since the read; `ready`. */
export type AboutStatus = "empty" | "unread" | "stale" | "ready";

export interface About {
  sources: Source[];
  status: AboutStatus;
  summary: Summary | null;
  /** Facts found in the documents that could not be traced, so left out. */
  leftOut: string[];
  /** When the profile was read (RFC 3339). */
  builtAt: string | null;
  limits: { maxChars: number; maxTotalChars: number };
}
