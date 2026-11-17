// Settings > Job sources, "Suggested sources": how a suggestion reads, and which steps of the
// `source_suggest` graph say how far the search is.

import type { GraphProgress } from "@/api/events";

import type { SourceSuggestion } from "@/types/discovery";

/** The graph of a "Suggest sources" search (`engine::discover::suggest::SUGGEST_GRAPH`). */
export const SUGGEST_GRAPH = "source_suggest";

/** `1 job`, `14 jobs`. */
function jobs(n: number): string {
  return n === 1 ? "1 job" : `${n} jobs`;
}

/** How many postings the source has and how many fit: `3 of 14 jobs fit your roles`; only `14 jobs`
 *  when all of them fit or none does (no roles to match, or nothing fits: the count says enough). */
export function fitText(s: Pick<SourceSuggestion, "jobs" | "matching">): string {
  if (s.matching <= 0 || s.matching >= s.jobs) {
    return jobs(s.jobs);
  }
  return `${s.matching} of ${s.jobs} jobs fit your roles`;
}

/** The name of a suggestion row: the name the check read, else its host, else its kind's label. */
export function suggestionName(s: Pick<SourceSuggestion, "name" | "host" | "label">): string {
  return s.name.trim() || s.host.trim() || s.label;
}

/** The line under the name: `Greenhouse · boards.greenhouse.io · 3 of 14 jobs fit your roles`. */
export function suggestionLine(s: SourceSuggestion): string {
  return [s.label, s.host.trim(), fitText(s)].filter((part) => part !== "").join(" · ");
}

/** The toast after "Add all": `1 source added`, `3 sources added`. */
export function addedText(n: number): string {
  return n === 1 ? "1 source added" : `${n} sources added`;
}

/** The progress line of a running search ("Searching 3 of 8", "Checking 5 sources"); `undefined`
 *  for a step that says nothing new. */
export function suggestNote(p: GraphProgress): string | undefined {
  if (p.graph !== SUGGEST_GRAPH) {
    return undefined;
  }
  return p.kind === "detail" && p.text !== null && p.text.trim() !== "" ? p.text : undefined;
}
