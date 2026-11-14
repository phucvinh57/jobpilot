// What the shell sends for the Trends screen (mirrors `src-tauri/src/trends/dto.rs`).
// Feed, page and posting text in these types is untrusted: it is rendered as text, never as HTML.

import type { Tone } from "@/shared";

/** The period a new run covers. */
export type Cadence = "weekly" | "quarterly";

/** The lists of the saved preferences a topic of the digest can be added to. */
export type PrefField = "skills" | "domains";

/** Where a topic stands in the saved preferences. */
export type Presence = "absent" | "added" | "avoided";

export interface Period {
  /** `2026-W46` / `2026-Q4`: what the run commands take. */
  label: string;
  /** `Week of 9 November` / `Q4 2026`. */
  title: string;
}

/** A stored item a point cites. */
export interface Citation {
  /** The item's position among the digest's cited items (1-based): the number on the chip. */
  number: number;
  title: string;
  /** Only a web address; `null`: nothing to open. */
  url: string | null;
}

export interface Suggest {
  field: PrefField;
  presence: Presence;
}

export interface Point {
  /** Position in the digest's points: the stable part of the element ids. */
  index: number;
  /** The badge; counts on through the sections. */
  number: number;
  topic: string;
  text: string;
  quote: string;
  citations: Citation[];
  /** A skill or domain the user can add to their preferences. */
  suggest: Suggest | null;
}

export interface DigestSection {
  title: string;
  points: Point[];
}

export type DigestBody =
  | { kind: "data"; sections: DigestSection[]; dropped: number }
  /** A report this version cannot read back: its text as saved. */
  | { kind: "plain"; lines: string[] };

export interface Digest {
  reportId: number;
  title: string;
  /** When it was made, in the user's time zone. */
  when: string;
  /** Items it read; `null` for a report this version cannot read back. */
  itemsRead: number | null;
  body: DigestBody;
}

export interface ChartSkill {
  name: string;
  total: number;
  /** The count, the share and its change, in words (the tooltip of the row). */
  description: string;
  presence: Presence;
}

export interface Chart {
  skills: ChartSkill[];
  /** The count the bars are scaled to. */
  max: number;
  /** Documents (postings and leads) read over the window. */
  documents: number;
  fromMonth: string;
  toMonth: string;
}

export interface ReportRow {
  id: number;
  title: string;
  when: string;
}

/** A collection paused at a login, captcha or block page. */
export interface Challenge {
  /** The period it collects for: what "Continue" resumes. */
  period: string;
  sourceName: string;
  kind: "login" | "captcha" | "blocked";
  /** `needs you to sign in` / `shows a captcha` / `blocks automated access`. */
  what: string;
  /** The page to open; `null` when it is not a web address. */
  url: string | null;
}

export interface Source {
  id: number;
  name: string;
  enabled: boolean;
  /** The kind and the host of its address (never the whole address). */
  description: string;
  /** What its last run says, in words. */
  statusText: string;
  tag: { label: string; tone: Tone };
}

export type SourceKind = "hackernews" | "yc" | "rss" | "page";

export interface SourceKindInfo {
  kind: SourceKind;
  label: string;
  /** A feed or page needs an address; Hacker News and YC do not. */
  needsUrl: boolean;
  /** Already a source (Hacker News and YC can be added once). */
  added: boolean;
}

export type SearchProvider = "brave" | "tavily";

export interface SearchKey {
  provider: SearchProvider;
  label: string;
  /** A key is in the OS keyring. The key itself never comes back. */
  saved: boolean;
}

/** Everything the screen shows, read in one call. */
export interface Overview {
  /** The current period of the chosen cadence: what "Collect & build digest" runs for. */
  period: Period;
  /** The line under the page title. */
  summary: string;
  digest: Digest | null;
  chart: Chart | null;
  /** Earlier digests, newest first. */
  history: ReportRow[];
  challenges: Challenge[];
  sources: Source[];
  sourceKinds: SourceKindInfo[];
  searchKeys: SearchKey[];
}

/** How a run that did not fail ended (a failed run is a `CommandError`). */
export interface RunResult {
  status: "done" | "waiting" | "cancelled";
  /** The report the run saved. */
  reportId: number | null;
  headline: string;
  /** One line per source, and per thing that went less well. */
  details: string[];
}

/** What "Add to preferences" did. */
export interface Added {
  presence: Presence;
  /** Why it was not added. */
  message: string | null;
}
