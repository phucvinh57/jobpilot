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

/** A label in a tone's colours, worked out by the shell. */
export interface TagInfo {
  label: string;
  tone: Tone;
}

/** Where a skill's share is going, decided in code. */
export type DirectionLabel = "rising" | "steady" | "falling" | "unclear";

/** A market slice: every posting, or the user's place. */
export type ScopeKey = "global" | "local";

export type GapBucket = "build_on" | "learn_next" | "at_risk";

export interface GapSkill {
  skill: string;
  /** `18%`: its share of the field's postings over the window. */
  share: string;
  direction: TagInfo & { label: DirectionLabel };
}

export interface GapGroup {
  bucket: GapBucket;
  /** `Build on` / `Learn next` / `At risk`. */
  title: string;
  /** What the bucket means. */
  hint: string;
  /** What the group says when it holds no skill. */
  empty: string;
  skills: GapSkill[];
}

/** The user's background against the market, sorted by the app (never by the AI). */
export interface Gap {
  scope: ScopeKey;
  /** Which market it was judged on. */
  hint: string;
  /** Build on, learn next, at risk: always the three, in that order. */
  groups: GapGroup[];
}

export interface Wish {
  wish: string;
  kind: "skill" | "domain";
  /** The user wants to avoid it rather than have it. */
  avoided: boolean;
  /** `rising` / `steady` / `falling` / `unclear` / `no data`. */
  signal: TagInfo;
  /** `42 postings` / `no posting names it`. */
  detail: string;
}

/** The user's wishes as the app measured them in the market. */
export interface Wishes {
  /** Which market they were measured on. */
  hint: string;
  wishes: Wish[];
}

export interface DigestSection {
  /** `rising_skills` ... `wishes_check`: the stable part of the element ids. */
  key: string;
  title: string;
  points: Point[];
  /** Only in `for_you`, when the report has a gap. */
  gap: Gap | null;
  /** Only in `wishes_check`, when the report has wishes. */
  wishes: Wishes | null;
}

/** A frequent topic the skill list does not know (third-party words: shown as text). */
export interface Term {
  term: string;
  /** Items whose topics name it. */
  items: number;
}

export type DigestBody =
  | { kind: "data"; sections: DigestSection[]; dropped: number; uncatalogued: Term[] }
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
  /** Its share of the scope's postings over the window, as a whole percent (the bar). */
  percent: number;
  /** The same share in words: `18%`, `2.5%`. */
  share: string;
  /** The count, the share and its change, in words (the tooltip of the row). */
  description: string;
  presence: Presence;
  /** Where its share is going; `null` for a report saved before the app measured it. */
  direction: (TagInfo & { label: DirectionLabel }) | null;
  /** In the user's profile. */
  inProfile: boolean;
  /** One the user wishes for (wanted, nice to have or avoided). */
  wished: boolean;
}

export interface Month {
  /** `2026-06`. */
  month: string;
  /** `Jun`. */
  label: string;
  /** Postings read that month. */
  documents: number;
}

/** One slice of the market's postings. */
export interface ChartScope {
  key: ScopeKey;
  /** `Local` / `Global`: the switch's choice. */
  label: string;
  /** What it covers: `Ho Chi Minh City, Singapore or remote` / `every posting`. */
  covers: string;
  /** Postings read over the window. */
  documents: number;
  months: Month[];
  skills: ChartSkill[];
  /** The percent the bars are scaled to. */
  maxPercent: number;
  /** What the scope says about its own data (too little local data, no direction yet). */
  note: string | null;
}

export interface Chart {
  /** Local first (when the report has a place), then global: the order of the switch. */
  scopes: ChartScope[];
  /** The scope shown first: the one the gap was judged on. */
  scope: ScopeKey;
  fromMonth: string;
  toMonth: string;
  /** The report knows the user's background: directions and the profile / wish marks are shown,
   *  and the bars highlight the profile (a report saved before highlights the preferences). */
  marks: boolean;
  /** The user's own leads, for comparison (never counted in the market). */
  pipeline: string | null;
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
